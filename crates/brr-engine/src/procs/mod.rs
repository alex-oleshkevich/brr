use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use brr_core::InstanceId;
use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::{Pid, getpgrp, getuid};

#[cfg(test)]
mod tests;

const PROC_DIR: &str = "/proc";
const MARKER_PREFIX: &[u8] = b"BRR_INSTANCE_ID=";

pub fn parse_environ(bytes: &[u8]) -> Option<InstanceId> {
    let value = bytes
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(MARKER_PREFIX))?;

    std::str::from_utf8(value).ok()?.parse().ok()
}

fn nix_pid(pid: u32) -> Pid {
    Pid::from_raw(pid as i32)
}

pub fn proc_pids() -> Vec<u32> {
    let Ok(entries) = fs::read_dir(PROC_DIR) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

// Chromium rewrites its main process environ, so only its children carry the marker.
pub fn marked_processes() -> Vec<(u32, InstanceId)> {
    let own_pid = std::process::id();
    let uid = getuid().as_raw();

    proc_pids()
        .into_iter()
        .filter(|pid| *pid != own_pid)
        .filter_map(|pid| {
            let dir = PathBuf::from(PROC_DIR).join(pid.to_string());
            if fs::metadata(&dir).ok()?.uid() != uid {
                return None;
            }
            let instance_id = parse_environ(&fs::read(dir.join("environ")).ok()?)?;

            Some((pid, instance_id))
        })
        .collect()
}

// (parent pid, process group) from /proc/<pid>/stat; the command name in field 2 may hold spaces
// and parentheses, so fields are counted after its closing parenthesis.
pub fn parent_and_group(pid: u32) -> Option<(u32, u32)> {
    let stat = fs::read(Path::new(PROC_DIR).join(pid.to_string()).join("stat")).ok()?;
    let stat = String::from_utf8_lossy(&stat);
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace().skip(1);

    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

pub fn descendants(pid: u32) -> Vec<u32> {
    descendants_in(Path::new(PROC_DIR), pid)
}

// `pid` and everything below it, from each thread's `children` file: no host-wide scan, so the
// cost follows the tree's size, not the number of processes on the machine.
pub fn descendants_in(proc_dir: &Path, pid: u32) -> Vec<u32> {
    let mut found = vec![pid];
    let mut next = 0;

    while let Some(parent) = found.get(next).copied() {
        next += 1;
        let Ok(tasks) = fs::read_dir(proc_dir.join(parent.to_string()).join("task")) else {
            continue;
        };

        for task in tasks.flatten() {
            let Ok(children) = fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            found.extend(
                children
                    .split_whitespace()
                    .filter_map(|child| child.parse::<u32>().ok()),
            );
        }
    }

    found
}

fn exe(pid: u32) -> Option<PathBuf> {
    fs::read_link(Path::new(PROC_DIR).join(pid.to_string()).join("exe")).ok()
}

// Kills every process carrying one of `ids`, the browser's main process (the parent of a marked
// child running the same binary) and the process groups of both.
pub fn kill_marked(ids: &HashSet<InstanceId>) {
    let own_pid = std::process::id();
    let own_group = getpgrp().as_raw() as u32;
    let mut victims = BTreeSet::new();

    for (pid, instance_id) in marked_processes() {
        if !ids.contains(&instance_id) {
            continue;
        }

        victims.insert(pid);
        if let Some((parent, _)) = parent_and_group(pid)
            && parent != own_pid
            && exe(parent).is_some_and(|parent_exe| Some(parent_exe) == exe(pid))
        {
            victims.insert(parent);
        }
    }

    for pid in victims {
        if let Some((_, group)) = parent_and_group(pid)
            && group != own_group
        {
            let _ = killpg(nix_pid(group), Signal::SIGKILL);
        }
        let _ = kill(nix_pid(pid), Signal::SIGKILL);
    }
}
