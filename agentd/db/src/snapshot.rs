//! Snapshots before data-changing migrations (docs/reference/db.md section 6): `VACUUM INTO`, permissions, "two latest" retention.
//! Nothing is deleted: extra snapshots are moved to `*.bak-YYYYMMDD`.

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::DbError;

const KEEP: usize = 2;
const PREFIX: &str = "pre-migration-";
const SUFFIX: &str = ".db";

/// Date `YYYYMMDD` in UTC from the system clock (civil calendar, Howard Hinnant's algorithm).
#[must_use]
pub fn today_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    stamp_from_unix(secs)
}

#[must_use]
pub fn stamp_from_unix(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
}

/// Free name `<path>.bak-<date>` (if taken: `-2`, `-3`…).
#[must_use]
pub fn backup_name(path: &Path, stamp: &str) -> PathBuf {
    let base = path.as_os_str().to_owned();
    let mut n = 1u32;
    loop {
        let mut s = base.clone();
        s.push(format!(".bak-{stamp}"));
        if n > 1 {
            s.push(format!("-{n}"));
        }
        let candidate = PathBuf::from(s);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn ensure_dir(dir: &Path) -> Result<(), DbError> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn snapshot_version(name: &str, prefix: &str) -> Option<i64> {
    name.strip_prefix(prefix)?
        .strip_suffix(SUFFIX)?
        .parse()
        .ok()
}

/// Takes `pre-migration-<version>.db` (directory `0700`, file `0600`), then applies retention.
pub fn take(conn: &Connection, dir: &Path, version: i64, stamp: &str) -> Result<PathBuf, DbError> {
    take_with(conn, dir, PREFIX, version, KEEP, stamp)
}

/// Snapshot `<prefix><number>.db` with "`keep` latest by number" retention; extras are moved to `*.bak-YYYYMMDD`.
pub fn take_with(
    conn: &Connection,
    dir: &Path,
    prefix: &str,
    number: i64,
    keep: usize,
    stamp: &str,
) -> Result<PathBuf, DbError> {
    ensure_dir(dir)?;
    let target = dir.join(format!("{prefix}{number}{SUFFIX}"));
    if target.exists() {
        fs::rename(&target, backup_name(&target, stamp))?;
    }
    // VACUUM INTO does not accept an existing file; until chmod the file is created per umask, but inside the 0700 directory it is inaccessible to others.
    let path_text = target.to_string_lossy().into_owned();
    conn.execute("VACUUM INTO ?1", [path_text])?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
    rotate_prefix(dir, prefix, keep, stamp)?;
    Ok(target)
}

/// "Two latest" retention for `pre-migration-*.db`: extras are moved to `*.bak-<stamp>`, nothing is deleted.
/// Returns how many files were moved. No directory: zero.
pub fn rotate(dir: &Path, stamp: &str) -> Result<u64, DbError> {
    rotate_prefix(dir, PREFIX, KEEP, stamp)
}

/// The same for any prefix: the `keep` latest by number stay, the rest go to `*.bak-<stamp>`.
pub fn rotate_prefix(dir: &Path, prefix: &str, keep: usize, stamp: &str) -> Result<u64, DbError> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut found: Vec<(i64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(v) = name.to_str().and_then(|n| snapshot_version(n, prefix)) {
            found.push((v, entry.path()));
        }
    }
    found.sort_by_key(|(v, _)| *v);
    let excess = found.len().saturating_sub(keep);
    let mut moved = 0u64;
    for (_, path) in found.into_iter().take(excess) {
        fs::rename(&path, backup_name(&path, stamp))?;
        moved += 1;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::stamp_from_unix;

    #[test]
    fn stamp_known_dates() {
        assert_eq!(stamp_from_unix(0), "19700101");
        assert_eq!(stamp_from_unix(1_700_000_000), "20231114");
        assert_eq!(stamp_from_unix(951_782_400), "20000229");
        assert_eq!(stamp_from_unix(-86_400), "19691231");
    }
}
