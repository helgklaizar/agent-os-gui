//! Encrypted session export (A6-11, docs/reference/db.md section 7): session and message JSON encrypted with `age` for the recipients
//! from settings, into `~/.agentos/export/` (`0700`, file `0600`). There is no unencrypted export: plaintext exists
//! only in process memory.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use agentd_core::{ExportError, ExportInfo, PortFuture, SessionExporter};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::Db;

pub struct ExportRepo {
    db: Arc<Db>,
    dir: PathBuf,
}

impl ExportRepo {
    /// `root` is `~/.agentos`; the export is written to `root/export`.
    #[must_use]
    pub fn new(db: Arc<Db>, root: impl Into<PathBuf>) -> ExportRepo {
        ExportRepo {
            db,
            dir: root.into().join("export"),
        }
    }
}

fn collect(c: &Connection, session_id: i64) -> rusqlite::Result<Option<Value>> {
    let Some(session) = crate::tree::session_json(c, session_id)? else {
        return Ok(None);
    };
    let messages = c
        .prepare("SELECT id, role, author, body_md, status, ts FROM messages WHERE session_id = ?1 ORDER BY id")?
        .query_map([session_id], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "role": r.get::<_, String>(1)?, "author": r.get::<_, Option<String>>(2)?,
                      "body_md": r.get::<_, String>(3)?, "status": r.get::<_, String>(4)?, "ts": r.get::<_, i64>(5)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(
        json!({"version": 1, "session": session, "messages": messages}),
    ))
}

fn encrypt(plain: &[u8], recipients: &[String]) -> Result<Vec<u8>, ExportError> {
    let parsed: Vec<age::x25519::Recipient> = recipients
        .iter()
        .map(|r| age::x25519::Recipient::from_str(r).map_err(|_| ExportError::BadRecipient))
        .collect::<Result<_, _>>()?;
    let enc = age::Encryptor::with_recipients(parsed.iter().map(|r| r as &dyn age::Recipient))
        .map_err(|_| ExportError::BadRecipient)?;
    let mut out = Vec::new();
    let mut w = enc
        .wrap_output(&mut out)
        .map_err(|_| ExportError::Unavailable)?;
    w.write_all(plain).map_err(|_| ExportError::Unavailable)?;
    w.finish().map_err(|_| ExportError::Unavailable)?;
    Ok(out)
}

fn write_private(dir: &PathBuf, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, dir.join(name))
    })();
    if result.is_err() {
        drop(fs::remove_file(&tmp));
    }
    result?;
    Ok(dir.join(name))
}

impl SessionExporter for ExportRepo {
    fn export(
        &self,
        session_id: i64,
        recipients: Vec<String>,
        now: i64,
    ) -> PortFuture<'_, Result<ExportInfo, ExportError>> {
        Box::pin(async move {
            // Recipients are validated before reading data: a bad key leaves no traces.
            encrypt(b"", &recipients)?;
            let doc = self
                .db
                .read(move |c| collect(c, session_id))
                .await
                .map_err(|_| ExportError::Unavailable)?
                .ok_or(ExportError::NotFound)?;
            let dir = self.dir.clone();
            tokio::task::spawn_blocking(move || {
                let cipher = encrypt(doc.to_string().as_bytes(), &recipients)?;
                let name = format!("session-{session_id}-{now}.json.age");
                let path =
                    write_private(&dir, &name, &cipher).map_err(|_| ExportError::Unavailable)?;
                Ok(ExportInfo {
                    path: path.to_string_lossy().into_owned(),
                    size: cipher.len() as u64,
                })
            })
            .await
            .map_err(|_| ExportError::Unavailable)?
        })
    }
}
