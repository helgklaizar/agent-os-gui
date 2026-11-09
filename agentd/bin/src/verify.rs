//! `agentd backup-verify` (S10-DB-02, automation A10): the archive is decrypted with the secret key, the contents are parsed,
//! `sessions.db` from the archive passes `integrity_check`, `foreign_key_check` and the `user_version` check. Any failure is an
//! `error` event in the feed (the cause without data values) and a non-zero exit code. The key is needed only for the duration of the check: it is not
//! on the working machine (ADR-0007), so the full check is done by the monthly restore test and the owner's manual run.

use std::path::Path;

use anyhow::{Result, bail};

use crate::archive::{DB_NAME, check_db_bytes, open_archive};

/// Returns the number of files in the archive.
pub fn run(archive: &Path, identity: &Path, db: &Path) -> Result<usize> {
    let checked = open_archive(archive, identity).and_then(|files| {
        let snap = files
            .iter()
            .find(|(n, _)| n == DB_NAME)
            .map(|(_, d)| d.as_slice())
            .unwrap_or_default();
        let dir = db.parent().unwrap_or_else(|| Path::new("."));
        check_db_bytes(snap, dir)?;
        Ok(files.len())
    });
    match checked {
        Ok(n) => Ok(n),
        Err(e) => {
            if let Err(w) = agentd_db::record_event(
                db,
                "error",
                &format!("Backup: archive failed verification: {e:#}"),
            ) {
                eprintln!("agentd: error event not recorded: {w}");
            }
            bail!("archive failed verification: {e:#}")
        }
    }
}
