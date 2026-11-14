use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use brr_core::{ConflictReason, Error};
use nix::{
    errno::Errno,
    fcntl::{Flock, FlockArg},
};

use crate::internal;

const LOCK_FILE_MODE: u32 = 0o600;

pub struct DaemonLock {
    _flock: Flock<File>,
}

impl DaemonLock {
    pub fn acquire(path: &Path) -> Result<DaemonLock, Error> {
        // Truncating at open would erase the running daemon's pid before the lock is known.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(LOCK_FILE_MODE)
            .open(path)
            .map_err(internal)?;

        let mut flock = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(flock) => flock,
            Err((_, Errno::EWOULDBLOCK)) => {
                return Err(Error::Conflict {
                    reason: ConflictReason::DaemonRunning,
                    detail: None,
                });
            }
            Err((_, errno)) => return Err(internal(errno)),
        };

        flock.set_len(0).map_err(internal)?;
        write!(flock, "{}", std::process::id()).map_err(internal)?;

        Ok(DaemonLock { _flock: flock })
    }
}
