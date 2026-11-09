//! DB files: directory `0700`, files `0600` (db.md section 1), single-writer lock, disk encryption check.

use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{DbError, StorageReport};
use crate::disk::{disk_encryption_status, disk_encryption_warning};

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const LOCK_ATTEMPTS: u32 = 20;
const LOCK_PAUSE: Duration = Duration::from_millis(10);

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn mode(path: &Path) -> std::io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

/// Sets the needed permissions; `true` if an already existing file or directory had to be tightened.
fn enforce(path: &Path, want: u32) -> Result<bool, DbError> {
    if mode(path)? == want {
        return Ok(false);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(want))?;
    Ok(true)
}

/// Exclusive writer lock. A short wait is needed when the previous writer has just closed and a duplicate
/// of the descriptor still lives in a child process between `fork` and `exec` (`lsattr` launch in a neighbouring thread).
fn take_writer_lock(lock: &File) -> Result<(), DbError> {
    for attempt in 0..LOCK_ATTEMPTS {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if attempt + 1 < LOCK_ATTEMPTS => {
                std::thread::sleep(LOCK_PAUSE)
            }
            Err(TryLockError::WouldBlock) => return Err(DbError::WriterBusy),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Err(DbError::WriterBusy)
}

/// Prepares the directory and files, takes the writer lock. Called before opening a connection.
pub(super) fn prepare(path: &Path) -> Result<File, DbError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(dir)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(FILE_MODE)
        .open(with_suffix(path, ".lock"))?;
    take_writer_lock(&lock)?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(FILE_MODE)
        .open(path)?;
    Ok(lock)
}

/// After migrations: permissions of `-wal`/`-shm`, directory and files, disk encryption check.
pub(super) fn finish(path: &Path) -> Result<StorageReport, DbError> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tightened = enforce(dir, DIR_MODE)?;
    for suffix in ["", ".lock", "-wal", "-shm"] {
        let p = with_suffix(path, suffix);
        if p.exists() {
            tightened |= enforce(&p, FILE_MODE)?;
        }
    }
    let encryption = disk_encryption_status(dir);
    let warning = disk_encryption_warning(encryption);
    if let Some(w) = warning {
        tracing::warn!(target: "fake_agentd::db", warning = ?w, "disk_encryption");
    }
    Ok(StorageReport {
        encryption,
        warning,
        perms_tightened: tightened,
    })
}
