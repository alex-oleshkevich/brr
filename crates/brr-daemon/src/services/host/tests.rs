use test_case::test_case;

use std::time::{Duration, Instant};

use brr_core::InstanceId;

use super::{
    PSS_TTL, PssCache, parse_cgroup_free, parse_cgroup_path, parse_loadavg, parse_meminfo,
    parse_pss_kb,
};

const MEMINFO: &str = "MemTotal:       16314508 kB\nMemFree:         1234567 kB\nMemAvailable:    8157254 kB\nBuffers:          123456 kB\n";
const SMAPS_ROLLUP: &str = "55d5c8a00000-7ffd4b5f9000 ---p 00000000 00:00 0                          [rollup]\nRss:               52340 kB\nPss:               31204 kB\nPss_Anon:          20000 kB\nPss_File:          11204 kB\n";
const TOTAL_KB: u64 = 16_314_508;
const AVAILABLE_KB: u64 = 8_157_254;
const LIMIT_BYTES: u64 = 536_870_912;
const CURRENT_BYTES: u64 = 100_000_000;

#[test]
fn meminfo_gives_total_and_available() {
    assert_eq!(parse_meminfo(MEMINFO), Some((TOTAL_KB, AVAILABLE_KB)));
}

#[test_case("MemTotal: 5 kB\n"; "no available")]
#[test_case("MemAvailable: 5 kB\n"; "no total")]
#[test_case(""; "empty")]
fn meminfo_needs_both_keys(text: &str) {
    assert_eq!(parse_meminfo(text), None);
}

#[test_case("0::/user.slice/user-1000.slice/session-2.scope\n", Some("/user.slice/user-1000.slice/session-2.scope"); "unified")]
#[test_case("12:memory:/old\n0::/\n", Some("/"); "hybrid keeps the unified line")]
#[test_case("12:memory:/old\n", None; "v1 only")]
fn cgroup_path(text: &str, expected: Option<&str>) {
    assert_eq!(parse_cgroup_path(text).as_deref(), expected);
}

#[test_case("max\n", "100\n", None; "unlimited")]
#[test_case("536870912\n", "100000000\n", Some(LIMIT_BYTES - CURRENT_BYTES); "limited")]
#[test_case("100\n", "200\n", Some(0); "over the limit")]
#[test_case("garbage", "1", None; "unreadable")]
fn cgroup_free(max: &str, current: &str, expected: Option<u64>) {
    assert_eq!(parse_cgroup_free(max, current), expected);
}

#[test_case("0.52 0.58 0.59 2/1234 56789\n", Some(0.52); "load")]
#[test_case("", None; "empty")]
fn loadavg(text: &str, expected: Option<f64>) {
    assert_eq!(parse_loadavg(text), expected);
}

#[test]
fn pss_ignores_the_breakdown_lines() {
    assert_eq!(parse_pss_kb(SMAPS_ROLLUP), Some(31_204));
    assert_eq!(parse_pss_kb("Rss: 1 kB\n"), None);
}

const BROWSER_PID: u32 = 4242;
const PSS_BYTES: u64 = 31_204 * 1024;

#[test]
fn a_sample_is_shared_until_the_ttl_passes() {
    let (mut cache, instance_id, sampled_at) =
        (PssCache::default(), InstanceId::new(), Instant::now());
    cache.store(instance_id, BROWSER_PID, PSS_BYTES, sampled_at);

    cache.expire(sampled_at + PSS_TTL - Duration::from_millis(1));
    assert_eq!(cache.fresh(&instance_id, BROWSER_PID), Some(PSS_BYTES));

    cache.expire(sampled_at + PSS_TTL);
    assert_eq!(cache.fresh(&instance_id, BROWSER_PID), None);
}

#[test]
fn a_relaunched_browser_is_measured_again() {
    let (mut cache, instance_id) = (PssCache::default(), InstanceId::new());
    cache.store(instance_id, BROWSER_PID, PSS_BYTES, Instant::now());

    assert_eq!(cache.fresh(&instance_id, BROWSER_PID + 1), None);
}
