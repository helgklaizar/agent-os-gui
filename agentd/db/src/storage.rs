//! DB files: directory `0700`, files `0600` (docs/reference/db.md sections 1, 2) and the instance lock (a second `agentd` on the same DB does not start).

use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::DbError;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const LOCK_ATTEMPTS: u32 = 20;
const LOCK_PAUSE: Duration = Duration::from_millis(10);

pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn mode_of(path: &Path) -> std::io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

/// Sets the needed permissions; `true` if an existing path had to be tightened.
fn enforce(path: &Path, want: u32) -> Result<bool, DbError> {
    if mode_of(path)? == want {
        return Ok(false);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(want))?;
    Ok(true)
}

pub(crate) fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Exclusive instance lock. A short wait is needed when the previous instance has just closed.
fn take_lock(lock: &File) -> Result<(), DbError> {
    for attempt in 0..LOCK_ATTEMPTS {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if attempt + 1 < LOCK_ATTEMPTS => {
                std::thread::sleep(LOCK_PAUSE)
            }
            Err(TryLockError::WouldBlock) => return Err(DbError::AlreadyRunning),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Err(DbError::AlreadyRunning)
}

fn open_private(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(FILE_MODE)
        .open(path)
}

/// Prepares the directory and files and takes the lock; called before the connection opens. The directory is created with `0700`;
/// an existing directory owned by our uid is tightened to `0700`, a foreign one (`/tmp`) is left alone.
pub(crate) fn prepare(path: &Path) -> Result<File, DbError> {
    let dir = parent_of(path);
    if !dir.exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(dir)?;
    } else if fs::metadata(dir)?.uid() == agentd_sys::current_uid() && enforce(dir, DIR_MODE)? {
        tracing::warn!(
            event = "db_dir_tightened",
            "DB directory permissions tightened to 0700"
        );
    }
    let lock = open_private(&with_suffix(path, ".lock"))?;
    take_lock(&lock)?;
    open_private(path)?;
    Ok(lock)
}

/// After migrations: permissions of the DB files, `-wal`, `-shm`, locks.
pub(crate) fn finish(path: &Path) -> Result<(), DbError> {
    for suffix in ["", ".lock", "-wal", "-shm"] {
        let p = with_suffix(path, suffix);
        if p.exists() && enforce(&p, FILE_MODE)? {
            tracing::warn!(
                event = "db_file_tightened",
                "DB file permissions tightened to 0600"
            );
        }
    }
    Ok(())
}
