use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_core::{Error, Profile, ProfileKind, QuotaScope};
use nix::sys::statvfs::statvfs;

use crate::{app::App, internal};

#[cfg(test)]
mod tests;

pub(crate) const BYTES_PER_GB: u64 = 1024 * 1024 * 1024;

pub async fn check_launch(app: &Arc<App>, profile: &Profile) -> Result<(), Error> {
    check_disk_floor(app)?;

    if profile.kind != ProfileKind::Persistent {
        return Ok(());
    }
    let (Some(quota), Some(data_dir)) = (profile.disk_quota_bytes, &profile.data_dir) else {
        return Ok(());
    };

    let used = dir_size(data_dir.clone()).await;
    if used > quota {
        return Err(Error::QuotaExceeded {
            scope: QuotaScope::Profile,
            used_bytes: used,
            limit_bytes: quota,
        });
    }

    Ok(())
}

pub fn floor_bytes(app: &App) -> u64 {
    app.settings().retention.free_disk_floor_gb * BYTES_PER_GB
}

pub fn check_disk_floor(app: &App) -> Result<(), Error> {
    let floor = floor_bytes(app);
    let free = free_bytes(&app.paths.data_dir)?;
    if free < floor {
        return Err(Error::QuotaExceeded {
            scope: QuotaScope::DiskFloor,
            used_bytes: free,
            limit_bytes: floor,
        });
    }

    Ok(())
}

pub fn free_bytes(path: &Path) -> Result<u64, Error> {
    let stat = statvfs(path).map_err(internal)?;

    Ok(stat.blocks_available() * stat.fragment_size())
}

// Symlinks are not followed: a link planted in a data dir must not count (or reach) files outside it.
pub async fn dir_size(path: PathBuf) -> u64 {
    tokio::task::spawn_blocking(move || {
        let mut total = 0;
        let mut pending = vec![path];

        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file() {
                    total += entry.metadata().map_or(0, |metadata| metadata.len());
                }
            }
        }

        total
    })
    .await
    .unwrap_or_default()
}
