#![allow(dead_code)]

use std::path::Path;

pub fn has_xvfb() -> bool {
    let found = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|dir| dir.join("Xvfb").is_file());
    if !found {
        eprintln!("skipped: Xvfb is not on PATH");
    }

    found
}

pub fn environ(pid: u32) -> String {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).unwrap();

    String::from_utf8_lossy(&bytes).replace('\0', "\n")
}

// Chromium overwrites its own main-process environ with the rewritten command line, so a variable
// is looked for in every process (its children inherit it). A process matches when each prefix
// starts one of its variables.
pub fn processes_with(prefixes: &[&str]) -> usize {
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path().join("environ")).ok())
        .filter(|environ| {
            prefixes.iter().all(|prefix| {
                environ
                    .split(|byte| *byte == 0)
                    .any(|var| var.starts_with(prefix.as_bytes()))
            })
        })
        .count()
}

pub fn exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}
