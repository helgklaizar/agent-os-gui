//! `agentd backup` (A7-09, S10-DB-01/02): a snapshot of `sessions.db`, `index.db`, `AGENT.md` and `memory/` in a single `tar` archive,
//! encrypted with `age` for recipients (primary and recovery). Recipients are the `--recipient` flags or `settings`
//! (`backup.recipient.primary` and `.recovery`, ADR-0007). The archive is `export/backup-<time>.tar.age` (`0700`/`0600`);
//! no plaintext archive ever exists, `keys.env` and other secrets are not included. The DB snapshot is verified before encryption,
//! the written file after it; a verification failure is an `error` event, the archive is not published (or is moved to `.corrupt-…`).
//! A second copy (`--copy-to`) goes to a different directory; the same disk does not count as a backup, so a warning is printed.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::archive::{DB_NAME, check_db_bytes, tar_entry, untar};

const MAX_FILE: u64 = 512 * 1024 * 1024;

fn read_plain(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && m.len() <= MAX_FILE => Ok(Some(fs::read(path)?)),
        _ => Ok(None),
    }
}

fn collect_memory(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(());
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        let m = fs::symlink_metadata(&p)?;
        if m.is_dir() {
            collect_memory(root, &p, out)?;
        } else if let (Some(data), Some(rel)) = (
            read_plain(&p)?,
            p.strip_prefix(root).ok().and_then(Path::to_str),
        ) {
            out.push((rel.to_string(), data));
        }
    }
    Ok(())
}

/// Recipients: flags, and without them both keys from `settings`; recovery is mandatory (ADR-0007 item 1).
fn resolve_recipients(db: &Path, flags: &[String]) -> Result<Vec<String>> {
    if !flags.is_empty() {
        return Ok(flags.to_vec());
    }
    let r = agentd_db::backup_recipients(db).context("recipient settings not read")?;
    match (r.primary, r.recovery) {
        (Some(p), Some(rec)) => Ok(vec![p, rec]),
        (None, _) => {
            bail!("no primary recipient: --recipient age1… or settings backup.recipient.primary")
        }
        (_, None) => bail!("no recovery recipient: no archive is created without it (ADR-0007)"),
    }
}

/// Failure after recipient checks: an `error` event into the feed and an error returned. The text contains no data values.
fn fail(db: &Path, what: &str) -> anyhow::Error {
    if let Err(e) = agentd_db::record_event(db, "error", &format!("Backup: {what}")) {
        eprintln!("agentd: error event not recorded: {e}");
    }
    anyhow::anyhow!("{what}")
}

fn digest(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// The written file is read back: the hash matches what went to disk, the `age` header parses.
fn verify_written(path: &Path, want: &[u8; 32]) -> Result<()> {
    let back = fs::read(path).context("archive not read back")?;
    if &digest(&back) != want {
        bail!("written archive does not match the source");
    }
    age::Decryptor::new(back.as_slice()).map_err(|_| anyhow::anyhow!("age header not parsed"))?;
    Ok(())
}

fn write_private(dir: &Path, name: &str, data: &[u8]) -> Result<PathBuf> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let tmp = dir.join(format!(".{name}.tmp"));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    let path = dir.join(name);
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Copy to a second location with a match check; `true` means a different disk (device), `false` the same one.
fn second_copy(archive: &Path, data: &[u8], dir: &Path) -> Result<(PathBuf, bool)> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .context("archive name")?;
    let copy = write_private(dir, name, data)?;
    if digest(&fs::read(&copy)?) != digest(data) {
        bail!("copy does not match the archive");
    }
    let other_disk = fs::metadata(&copy)?.dev() != fs::metadata(archive)?.dev();
    Ok((copy, other_disk))
}

/// Returns the archive path and, if given, the path of the second copy.
pub fn run(
    db: &Path,
    root: &Path,
    recipients: &[String],
    copy_to: Option<&Path>,
) -> Result<Vec<PathBuf>> {
    let parsed: Vec<age::x25519::Recipient> = resolve_recipients(db, recipients)?
        .iter()
        .map(|r| {
            age::x25519::Recipient::from_str(r)
                .map_err(|_| anyhow::anyhow!("recipient not parsed as an age key"))
        })
        .collect::<Result<_>>()?;
    let snapshot = agentd_db::backup_snapshot(db).context("sessions.db snapshot")?;
    check_db_bytes(&snapshot, root).map_err(|e| {
        fail(
            db,
            &format!("DB failed verification, archive not created: {e}"),
        )
    })?;
    let mut files: Vec<(String, Vec<u8>)> = vec![(DB_NAME.into(), snapshot)];
    if let Some(i) = read_plain(&root.join("index.db"))? {
        files.push(("index.db".into(), i));
    }
    if let Some(a) = read_plain(&root.join("AGENT.md"))? {
        files.push(("AGENT.md".into(), a));
    }
    collect_memory(root, &root.join("memory"), &mut files)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut tar = Vec::new();
    for (name, data) in &files {
        tar_entry(&mut tar, name, data, now)?;
    }
    tar.extend_from_slice(&[0u8; 1024]);
    if untar(&tar)?.len() != files.len() {
        return Err(fail(
            db,
            "archive contents did not match when parsed, archive not created",
        ));
    }
    let enc = age::Encryptor::with_recipients(parsed.iter().map(|r| r as &dyn age::Recipient))
        .context("recipients")?;
    let mut cipher = Vec::new();
    let mut w = enc.wrap_output(&mut cipher).context("encryption")?;
    w.write_all(&tar)?;
    w.finish()?;
    let name = format!("backup-{now}.tar.age");
    let path = write_private(&root.join("export"), &name, &cipher)?;
    if let Err(e) = verify_written(&path, &digest(&cipher)) {
        // A corrupt file is not deleted, but it does not stay under the archive name either.
        let bad = path.with_file_name(format!(".corrupt-{name}"));
        drop(fs::rename(&path, bad));
        return Err(fail(
            db,
            &format!("written archive failed verification: {e}"),
        ));
    }
    let mut out = vec![path.clone()];
    if let Some(dir) = copy_to {
        let (copy, other_disk) = second_copy(&path, &cipher, dir)
            .map_err(|e| fail(db, &format!("second copy not created: {e}")))?;
        if !other_disk {
            eprintln!(
                "agentd: a copy on the same disk does not count as a backup (docs/reference/db.md section 7): an external medium is needed"
            );
        }
        out.push(copy);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{digest, verify_written};

    fn archive_bytes() -> Vec<u8> {
        let id = age::x25519::Identity::generate();
        let enc = age::Encryptor::with_recipients(std::iter::once(
            &id.to_public() as &dyn age::Recipient
        ))
        .unwrap();
        let mut out = Vec::new();
        let mut w = enc.wrap_output(&mut out).unwrap();
        std::io::Write::write_all(&mut w, b"payload").unwrap();
        w.finish().unwrap();
        out
    }

    #[test]
    fn verify_written_accepts_intact_and_rejects_changed_or_headerless_files() {
        let dir = tempfile::tempdir().unwrap();
        let good = archive_bytes();
        let p = dir.path().join("a.tar.age");
        std::fs::write(&p, &good).unwrap();
        assert!(verify_written(&p, &digest(&good)).is_ok());
        let mut bad = good.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        std::fs::write(&p, &bad).unwrap();
        assert!(verify_written(&p, &digest(&good)).is_err(), "byte changed");
        std::fs::write(&p, b"not age").unwrap();
        assert!(
            verify_written(&p, &digest(b"not age")).is_err(),
            "hash is correct, no age header"
        );
    }
}
