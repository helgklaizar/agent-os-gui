//! `~/.agentos/keys.env`: secret value store. File `0600`, directory `0700`, atomic write
//! (temporary file in the same directory, `rename`). Foreign lines of the file (comments, other variables) are preserved.

use crate::keys_format::{Line, parse, render, var_name};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agentd_core::{
    MAX_SECRET_BYTES, PortFuture, Secret, SecretEraser, SecretError, SecretInfo, SecretStore,
    valid_secret_ref,
};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// File-backed store; values never appear in `Debug`.
#[derive(Debug, Clone)]
pub struct KeysFile {
    path: PathBuf,
    // Writes are serialized: read-modify-write of the file must not interleave.
    lock: Arc<Mutex<()>>,
}

fn io_err(e: &std::io::Error) -> SecretError {
    tracing::warn!(scope = "secrets", event = "keys_io", kind = ?e.kind(), "keys.env unavailable");
    SecretError::Unavailable
}

impl KeysFile {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> KeysFile {
        KeysFile {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<Vec<Line>, SecretError> {
        match fs::read_to_string(&self.path) {
            Ok(t) => Ok(parse(&t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io_err(&e)),
        }
    }

    /// Directory `0700` (an existing one of ours is tightened), temporary file `0600`, `fsync`, `rename`.
    fn store(&self, lines: &[Line]) -> Result<(), SecretError> {
        let dir = self
            .path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if dir.exists() {
            let meta = fs::metadata(dir).map_err(|e| io_err(&e))?;
            if meta.uid() == agentd_sys::current_uid() && meta.permissions().mode() & 0o077 != 0 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                    .map_err(|e| io_err(&e))?;
            }
        } else {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| io_err(&e))?;
        }
        let tmp = dir.join(format!(
            ".keys.env.tmp-{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(render(lines).as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, &self.path)?;
            fs::File::open(dir)?.sync_all()
        })();
        if let Err(e) = written {
            drop(fs::remove_file(&tmp));
            return Err(io_err(&e));
        }
        Ok(())
    }

    fn with_lock<T>(&self, f: impl FnOnce() -> T) -> T {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn put_sync(&self, secret_ref: &str, value: &str, now: i64) -> Result<(), SecretError> {
        if !valid_secret_ref(secret_ref) {
            return Err(SecretError::Invalid("ref"));
        }
        if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains('\0') {
            return Err(SecretError::Invalid("value"));
        }
        self.with_lock(|| {
            let mut lines = self.load()?;
            let name = var_name(secret_ref);
            let clash = lines.iter().any(|l| {
                matches!(l, Line::Entry { secret_ref: r, .. } if r != secret_ref && var_name(r) == name)
            });
            if clash {
                return Err(SecretError::Invalid("ref"));
            }
            let entry = Line::Entry {
                secret_ref: secret_ref.to_string(),
                updated_at: now,
                value: value.to_string(),
            };
            match lines
                .iter_mut()
                .find(|l| matches!(l, Line::Entry { secret_ref: r, .. } if r == secret_ref))
            {
                Some(slot) => *slot = entry,
                None => lines.push(entry),
            }
            self.store(&lines)
        })
    }

    fn delete_sync(&self, secret_ref: &str) -> Result<bool, SecretError> {
        self.with_lock(|| {
            let mut lines = self.load()?;
            let before = lines.len();
            lines.retain(|l| !matches!(l, Line::Entry { secret_ref: r, .. } if r == secret_ref));
            if lines.len() == before {
                return Ok(false);
            }
            self.store(&lines)?;
            Ok(true)
        })
    }
}

impl SecretStore for KeysFile {
    fn list(&self) -> PortFuture<'_, Result<Vec<SecretInfo>, SecretError>> {
        Box::pin(async move {
            let me = self.clone();
            let lines = run(move || me.with_lock(|| me.load())).await??;
            Ok(lines
                .into_iter()
                .filter_map(|l| match l {
                    Line::Entry {
                        secret_ref,
                        updated_at,
                        ..
                    } => Some(SecretInfo {
                        secret_ref,
                        set: true,
                        updated_at,
                    }),
                    Line::Foreign(_) => None,
                })
                .collect())
        })
    }

    fn put<'a>(
        &'a self,
        secret_ref: &'a str,
        value: Secret<String>,
        now: i64,
    ) -> PortFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            let (me, r) = (self.clone(), secret_ref.to_string());
            run(move || me.put_sync(&r, value.expose(), now)).await?
        })
    }

    fn delete<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, Result<bool, SecretError>> {
        Box::pin(async move {
            let (me, r) = (self.clone(), secret_ref.to_string());
            run(move || me.delete_sync(&r)).await?
        })
    }

    fn get<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, Option<Secret<String>>> {
        Box::pin(async move {
            let (me, r) = (self.clone(), secret_ref.to_string());
            let lines = run(move || me.with_lock(|| me.load())).await.ok()?.ok()?;
            lines.into_iter().find_map(|l| match l {
                Line::Entry {
                    secret_ref, value, ..
                } if secret_ref == r => Some(Secret::new(value)),
                _ => None,
            })
        })
    }
}

impl SecretEraser for KeysFile {
    fn erase<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, ()> {
        Box::pin(async move {
            let (me, r) = (self.clone(), secret_ref.to_string());
            match run(move || me.delete_sync(&r)).await {
                Ok(Ok(_)) => {}
                _ => tracing::warn!(
                    scope = "secrets",
                    event = "erase_failed",
                    "account secret not erased"
                ),
            }
        })
    }
}

async fn run<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, SecretError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| SecretError::Unavailable)
}
