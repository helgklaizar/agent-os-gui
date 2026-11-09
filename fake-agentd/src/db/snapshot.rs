//! Snapshot before a data-changing migration (db.md section 6): `VACUUM INTO <DB dir>/migration-backups/pre-migration-VERSION.db`.
//! Directory `0700`, file `0600`; the last two are kept, older ones are moved to `*.bak-YYYYMMDD` (not deleted).
//!
//! Which migrations change data is decided by the `Migration::mutates_data` field (as in the core catalog), not by the SQL text.

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::DbError;

const KEEP: usize = 2;
const PREFIX: &str = "pre-migration-";
const SUFFIX: &str = ".db";

/// Snapshot directory next to the DB file; `None` for an in-memory DB.
fn backup_dir(conn: &Connection) -> Option<PathBuf> {
    let path = Path::new(conn.path()?);
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    path.file_name()?;
    Some(parent.join("migration-backups"))
}

/// Date `YYYYMMDD` in UTC (civil calendar, Hinnant's algorithm).
fn today_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
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

/// Free name `<path>.bak-<date>` (if taken - `-2`, `-3`...).
fn backup_name(path: &Path, stamp: &str) -> PathBuf {
    let mut n = 1u32;
    loop {
        let mut s = path.as_os_str().to_owned();
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

fn version_of(name: &str) -> Option<i64> {
    name.strip_prefix(PREFIX)?
        .strip_suffix(SUFFIX)?
        .parse()
        .ok()
}

/// Takes `pre-migration-<version>.db` if the DB is a file; returns the snapshot path.
pub fn take(conn: &Connection, version: i64) -> Result<Option<PathBuf>, DbError> {
    let Some(dir) = backup_dir(conn) else {
        return Ok(None);
    };
    let stamp = today_stamp();
    DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let target = dir.join(format!("{PREFIX}{version}{SUFFIX}"));
    if target.exists() {
        fs::rename(&target, backup_name(&target, &stamp))?;
    }
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy().into_owned()])?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
    let mut found: Vec<(i64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if let Some(v) = entry.file_name().to_str().and_then(version_of) {
            found.push((v, entry.path()));
        }
    }
    found.sort_by_key(|(v, _)| *v);
    let excess = found.len().saturating_sub(KEEP);
    for (_, old) in found.into_iter().take(excess) {
        fs::rename(&old, backup_name(&old, &stamp))?;
    }
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_has_eight_digits() {
        let s = today_stamp();
        assert_eq!(s.len(), 8);
        assert!(s.chars().all(|c| c.is_ascii_digit()));
    }
}
