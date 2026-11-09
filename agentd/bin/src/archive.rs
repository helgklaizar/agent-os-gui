//! Shared by `backup`, `backup-verify` and `restore`: writing and parsing ustar, archive decryption, verification of the DB from the archive
//! (`integrity_check`, `foreign_key_check`, `user_version` against `schema_migrations`, S10-DB-05).

use std::fs;
use std::io::Read;
use std::path::{Component, Path};
use std::str::FromStr;

use anyhow::{Context, Result, bail};

pub const DB_NAME: &str = "sessions.db";

/// ustar header (regular file) and aligned content.
pub fn tar_entry(out: &mut Vec<u8>, name: &str, data: &[u8], mtime: u64) -> Result<()> {
    if name.len() > 100 {
        bail!("name in archive longer than 100 bytes: {name}");
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000600\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
    h[136..148].copy_from_slice(format!("{mtime:011o}\0").as_bytes());
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(data);
    out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    Ok(())
}

/// ustar parsing as written by `backup`: regular files only; names are checked.
pub fn untar(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 512 <= data.len() && data[i] != 0 {
        let name = String::from_utf8_lossy(&data[i..i + 100])
            .trim_end_matches('\0')
            .to_string();
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&data[i + 124..i + 135]).trim_matches(['\0', ' ']),
            8,
        )
        .context("archive entry size")?;
        let end = i
            .checked_add(512)
            .and_then(|s| s.checked_add(size))
            .filter(|e| *e <= data.len())
            .context("archive truncated")?;
        if data[i + 156] != b'0' && data[i + 156] != 0 {
            bail!("archive entry is not a regular file: {name}");
        }
        check_name(&name)?;
        out.push((name, data[i + 512..end].to_vec()));
        i += 512 + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// Only what `backup` puts in is allowed; `..`, absolute paths and extra names are rejected.
fn check_name(name: &str) -> Result<()> {
    let ok_path = Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    let known = matches!(name, DB_NAME | "index.db" | "AGENT.md")
        || name
            .strip_prefix("memory/")
            .is_some_and(|rest| !rest.is_empty());
    if !ok_path || !known {
        bail!("invalid name in archive: {name}");
    }
    Ok(())
}

/// `age` secret key from an `age-keygen` file: the first non-comment line. The content never appears in error messages.
pub fn read_identity(path: &Path) -> Result<age::x25519::Identity> {
    let text = fs::read_to_string(path).context("key file not read")?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .context("no key line in the key file")?;
    age::x25519::Identity::from_str(line)
        .map_err(|_| anyhow::anyhow!("key not parsed as an age secret key"))
}

/// Decrypts the archive to the end (damage to any block gives an authentication error) and parses the contents.
pub fn open_archive(archive: &Path, identity: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let id = read_identity(identity)?;
    let dec = age::Decryptor::new(fs::File::open(archive).context("archive did not open")?)
        .map_err(|_| anyhow::anyhow!("file does not look like an age archive"))?;
    let mut tar = Vec::new();
    dec.decrypt(std::iter::once(&id as &dyn age::Identity))
        .map_err(|_| anyhow::anyhow!("archive not decrypted with this key"))?
        .read_to_end(&mut tar)
        .context("archive corrupted: read interrupted")?;
    let files = untar(&tar)?;
    if !files.iter().any(|(n, _)| n == DB_NAME) {
        bail!("no {DB_NAME} in the archive");
    }
    Ok(files)
}

/// DB file check (S10-DB-05); the error lists all problems found.
pub fn check_db_file(path: &Path) -> Result<()> {
    let report = agentd_db::check_file(path).context("DB did not open for verification")?;
    let problems = report.problems();
    if !problems.is_empty() {
        bail!("{}", problems.join("; "));
    }
    Ok(())
}

/// DB check from memory: into a `0700` temporary directory next to `dir`, the file is removed by the library on exit.
pub fn check_db_bytes(bytes: &[u8], dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = tempfile::Builder::new()
        .prefix(".backup-check-")
        .tempdir_in(dir)?;
    let file = tmp.path().join("check.db");
    fs::write(&file, bytes)?;
    check_db_file(&file)
}
