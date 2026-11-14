use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

use brr_core::{Error, InstanceId};

use crate::internal;
use brr_engine::procs::descendants;
use tokio::sync::Mutex;

#[cfg(test)]
mod tests;

const PSS_TTL: Duration = Duration::from_secs(5);
const MEMINFO: &str = "/proc/meminfo";
const LOADAVG: &str = "/proc/loadavg";
const SELF_CGROUP: &str = "/proc/self/cgroup";
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const CGROUP_V2_PREFIX: &str = "0::";
const MEM_TOTAL_KEY: &str = "MemTotal:";
const MEM_AVAILABLE_KEY: &str = "MemAvailable:";
const PSS_KEY: &str = "Pss:";
const UNLIMITED: &str = "max";
const KB_PER_MB: u64 = 1024;
const BYTES_PER_KB: u64 = 1024;
pub const BYTES_PER_MB: u64 = 1024 * 1024;

static PSS_CACHE: LazyLock<Mutex<PssCache>> = LazyLock::new(Mutex::default);

pub struct Memory {
    pub total_mb: u64,
    pub available_mb: u64,
}

// A container's MemAvailable reports the host, so the cgroup's own headroom caps it.
pub fn memory() -> Result<Memory, Error> {
    let meminfo = fs::read_to_string(MEMINFO).map_err(internal)?;
    let (total_kb, available_kb) = parse_meminfo(&meminfo)
        .ok_or_else(|| internal(format!("cannot read memory from {MEMINFO}")))?;

    let mut available_mb = available_kb / KB_PER_MB;
    if let Some(free_bytes) = cgroup_free_bytes() {
        available_mb = available_mb.min(free_bytes / BYTES_PER_MB);
    }

    Ok(Memory {
        total_mb: total_kb / KB_PER_MB,
        available_mb,
    })
}

pub fn load_per_core() -> Result<f64, Error> {
    let loadavg = fs::read_to_string(LOADAVG).map_err(internal)?;
    let load_1m = parse_loadavg(&loadavg)
        .ok_or_else(|| internal(format!("cannot read the load average from {LOADAVG}")))?;
    let cpus = thread::available_parallelism().map_err(internal)?.get();

    Ok(load_1m / cpus as f64)
}

// Pss, not RSS: pages shared between a browser's processes count once. An Instance's processes
// are its browser pid and everything below it. Pss changes slowly and costs a /proc read per
// process, so every caller (list, get, /metrics, admission) shares one sample per Instance for
// PSS_TTL, and the lock makes concurrent callers wait for the sample instead of repeating it.
pub async fn pss_by_instance(browsers: Vec<(InstanceId, u32)>) -> HashMap<InstanceId, u64> {
    let mut cache = PSS_CACHE.lock().await;
    let now = Instant::now();
    cache.expire(now);

    let mut sums = HashMap::new();
    let mut stale = Vec::new();
    for (instance_id, pid) in browsers {
        match cache.fresh(&instance_id, pid) {
            Some(bytes) => {
                sums.insert(instance_id, bytes);
            }
            None => stale.push((instance_id, pid)),
        }
    }
    if stale.is_empty() {
        return sums;
    }

    let measured = tokio::task::spawn_blocking(move || measure(&stale))
        .await
        .unwrap_or_default();
    let sampled_at = Instant::now();
    for (instance_id, (pid, bytes)) in measured {
        cache.store(instance_id, pid, bytes, sampled_at);
        sums.insert(instance_id, bytes);
    }

    sums
}

#[derive(Default)]
struct PssCache {
    entries: HashMap<InstanceId, (u32, Instant, u64)>,
}

impl PssCache {
    // A relaunched Instance has a new pid and must be measured again.
    fn fresh(&self, instance_id: &InstanceId, pid: u32) -> Option<u64> {
        let (cached_pid, _, bytes) = self.entries.get(instance_id)?;

        (*cached_pid == pid).then_some(*bytes)
    }

    fn store(&mut self, instance_id: InstanceId, pid: u32, bytes: u64, now: Instant) {
        self.entries.insert(instance_id, (pid, now, bytes));
    }

    fn expire(&mut self, now: Instant) {
        self.entries
            .retain(|_, (_, sampled_at, _)| now.saturating_duration_since(*sampled_at) < PSS_TTL);
    }
}

fn measure(browsers: &[(InstanceId, u32)]) -> HashMap<InstanceId, (u32, u64)> {
    browsers
        .iter()
        .map(|(instance_id, browser_pid)| {
            let bytes = descendants(*browser_pid)
                .into_iter()
                .filter_map(pss_bytes)
                .sum();

            (*instance_id, (*browser_pid, bytes))
        })
        .collect()
}

// A process may exit mid-scan.
fn pss_bytes(pid: u32) -> Option<u64> {
    let rollup = Path::new("/proc")
        .join(pid.to_string())
        .join("smaps_rollup");
    let pss_kb = parse_pss_kb(&fs::read_to_string(rollup).ok()?)?;

    Some(pss_kb * BYTES_PER_KB)
}

// The tightest limit on the way up: a container's limit sits on a parent of the daemon's own
// cgroup, not on the cgroup itself.
fn cgroup_free_bytes() -> Option<u64> {
    let path = parse_cgroup_path(&fs::read_to_string(SELF_CGROUP).ok()?)?;
    let root = Path::new(CGROUP_ROOT);
    let leaf = root.join(path.trim_start_matches('/'));

    leaf.ancestors()
        .take_while(|dir| *dir != root)
        .filter_map(|dir| {
            parse_cgroup_free(
                &fs::read_to_string(dir.join("memory.max")).ok()?,
                &fs::read_to_string(dir.join("memory.current")).ok()?,
            )
        })
        .min()
}

// (MemTotal, MemAvailable) in kB.
pub fn parse_meminfo(meminfo: &str) -> Option<(u64, u64)> {
    let find = |key: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(key))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };

    Some((find(MEM_TOTAL_KEY)?, find(MEM_AVAILABLE_KEY)?))
}

pub fn parse_cgroup_path(cgroup: &str) -> Option<String> {
    cgroup
        .lines()
        .find_map(|line| line.strip_prefix(CGROUP_V2_PREFIX))
        .map(str::to_owned)
}

// None when the cgroup has no limit.
pub fn parse_cgroup_free(max: &str, current: &str) -> Option<u64> {
    let max = max.trim();
    if max == UNLIMITED {
        return None;
    }

    Some(
        max.parse::<u64>()
            .ok()?
            .saturating_sub(current.trim().parse().ok()?),
    )
}

pub fn parse_loadavg(loadavg: &str) -> Option<f64> {
    loadavg.split_whitespace().next()?.parse().ok()
}

pub fn parse_pss_kb(smaps_rollup: &str) -> Option<u64> {
    smaps_rollup
        .lines()
        .find_map(|line| line.strip_prefix(PSS_KEY))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}
