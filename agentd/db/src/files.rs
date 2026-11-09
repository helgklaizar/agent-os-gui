//! `FileStore` (A6-10): `AGENT.md` and `memory/` files under `~/.agentos`. Symlinks are not dereferenced on any
//! path segment, writes are atomic (a `0600` temporary file and rename), directories are created with `0700`.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agentd_core::{
    FileError, FileInfo, FilePath, FileStore, MAX_FILE_BYTES, MISSING_REV, PortFuture, normalize,
};
use sha2::{Digest, Sha256};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct FileRepo {
    root: PathBuf,
}

impl FileRepo {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> FileRepo {
        FileRepo { root: root.into() }
    }
}

fn rev_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn io(e: &std::io::Error) -> FileError {
    tracing::warn!(scope = "files", event = "io_error", kind = ?e.kind(), "file operation error");
    FileError::Unavailable
}

/// Full path; no existing segment may be a link.
fn resolve(root: &Path, rel: &str) -> Result<PathBuf, FileError> {
    let mut cur = root.to_path_buf();
    for comp in rel.split('/') {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(FileError::Denied),
            Ok(_) | Err(_) => {}
        }
    }
    Ok(cur)
}

fn read_regular(path: &Path) -> Result<Option<Vec<u8>>, FileError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => {
            if m.len() > MAX_FILE_BYTES as u64 {
                return Err(FileError::TooLarge);
            }
            let mut buf = Vec::new();
            File::open(path)
                .and_then(|mut f| f.read_to_end(&mut buf))
                .map_err(|e| io(&e))?;
            Ok(Some(buf))
        }
        Ok(_) => Err(FileError::Denied),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(&e)),
    }
}

fn info(path: &FilePath, bytes: &[u8]) -> FileInfo {
    FileInfo {
        path: path.rel.clone(),
        kind: path.kind,
        size: bytes.len() as u64,
        rev: rev_of(bytes),
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<FilePath>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = fs::symlink_metadata(&p) else {
            continue;
        };
        if m.is_dir() {
            collect(root, &p, out);
        } else if m.is_file()
            && let Some(rel) = p.strip_prefix(root).ok().and_then(Path::to_str)
            && let Some(fp) = normalize(rel)
        {
            out.push(fp);
        }
    }
}

fn list(root: &Path) -> Result<Vec<FileInfo>, FileError> {
    let mut paths = Vec::new();
    if let Some(fp) = normalize("AGENT.md") {
        paths.push(fp);
    }
    let mut mem = Vec::new();
    collect(root, &root.join("memory"), &mut mem);
    mem.sort_by(|a, b| a.rel.cmp(&b.rel));
    paths.extend(mem);
    let mut out = Vec::new();
    for p in paths {
        // Links and files that are too large are not listed.
        let Ok(full) = resolve(root, &p.rel) else {
            continue;
        };
        match read_regular(&full) {
            Ok(Some(bytes)) => out.push(info(&p, &bytes)),
            Ok(None) | Err(FileError::TooLarge | FileError::Denied) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

fn make_dirs(root: &Path, rel: &str) -> Result<(), FileError> {
    let mut cur = root.to_path_buf();
    let comps: Vec<&str> = rel.split('/').collect();
    for comp in &comps[..comps.len() - 1] {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(FileError::Denied),
            Err(_) => DirBuilder::new()
                .mode(0o700)
                .create(&cur)
                .map_err(|e| io(&e))?,
        }
    }
    Ok(())
}

fn write(root: &Path, p: &FilePath, if_match: &str, body: &[u8]) -> Result<FileInfo, FileError> {
    if body.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge);
    }
    let full = resolve(root, &p.rel)?;
    let current = match read_regular(&full) {
        Ok(Some(b)) => rev_of(&b),
        Ok(None) => MISSING_REV.to_string(),
        Err(FileError::TooLarge) => "oversized".to_string(),
        Err(e) => return Err(e),
    };
    if current != if_match {
        return Err(FileError::Stale(current));
    }
    make_dirs(root, &p.rel)?;
    let dir = full.parent().ok_or(FileError::Denied)?;
    let name = full.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        fs::rename(&tmp, &full)
    })();
    if let Err(e) = written {
        drop(fs::remove_file(&tmp));
        return Err(io(&e));
    }
    Ok(info(p, body))
}

impl FileStore for FileRepo {
    fn list(&self) -> PortFuture<'_, Result<Vec<FileInfo>, FileError>> {
        let root = self.root.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || list(&root))
                .await
                .map_err(|_| FileError::Unavailable)?
        })
    }

    fn read(&self, path: FilePath) -> PortFuture<'_, Result<(Vec<u8>, FileInfo), FileError>> {
        let root = self.root.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let full = resolve(&root, &path.rel)?;
                let bytes = read_regular(&full)?.ok_or(FileError::NotFound)?;
                let i = info(&path, &bytes);
                Ok((bytes, i))
            })
            .await
            .map_err(|_| FileError::Unavailable)?
        })
    }

    fn write(
        &self,
        path: FilePath,
        if_match: String,
        body: Vec<u8>,
    ) -> PortFuture<'_, Result<FileInfo, FileError>> {
        let root = self.root.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || write(&root, &path, &if_match, &body))
                .await
                .map_err(|_| FileError::Unavailable)?
        })
    }
}
