//! Attachments: file `<id>` (`0600`) and description `<id>.json` in `~/.agentos/attachments/` (directory `0700`). The user's
//! file name is stored as data; the path is built only from the numeric `id`.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentd_core::{Attachment, ChatError, MAX_ATTACHMENT_BYTES};
use serde_json::{Value, json};

use super::messages::invalid;

pub(crate) struct Attachments {
    dir: PathBuf,
    lock: Mutex<()>,
}

fn io(e: &std::io::Error) -> ChatError {
    tracing::warn!(scope = "chat", event = "attachment_io", kind = ?e.kind(), "attachment storage unavailable");
    ChatError::Unavailable
}

/// Name as data: no control characters, at most 255 characters.
fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).take(255).collect();
    if n.trim().is_empty() {
        "file".into()
    } else {
        n
    }
}

fn clean_mime(mime: &str) -> String {
    let ok = mime.len() <= 100
        && mime.split_once('/').is_some_and(|(a, b)| {
            let tok = |s: &str| {
                !s.is_empty()
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&c))
            };
            tok(a) && tok(b)
        });
    if ok {
        mime.to_ascii_lowercase()
    } else {
        "application/octet-stream".into()
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let r = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if r.is_err() {
        drop(fs::remove_file(&tmp));
    }
    r
}

impl Attachments {
    pub(crate) fn new(dir: PathBuf) -> Attachments {
        Attachments {
            dir,
            lock: Mutex::new(()),
        }
    }

    fn next_id(&self) -> std::io::Result<i64> {
        let seq = self.dir.join(".seq");
        let last: i64 = fs::read_to_string(&seq)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        write_atomic(&seq, (last + 1).to_string().as_bytes())?;
        Ok(last + 1)
    }

    pub(crate) fn put(
        &self,
        session_id: i64,
        name: &str,
        mime: &str,
        data: &[u8],
    ) -> Result<Attachment, ChatError> {
        if data.len() > MAX_ATTACHMENT_BYTES {
            return Err(ChatError::TooLarge);
        }
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(|e| io(&e))?;
        let id = self.next_id().map_err(|e| io(&e))?;
        let att = Attachment {
            id,
            name: clean_name(name),
            mime: clean_mime(mime),
            size: data.len() as u64,
        };
        let info =
            json!({"name": att.name, "mime": att.mime, "size": att.size, "session_id": session_id});
        write_atomic(&self.dir.join(id.to_string()), data).map_err(|e| io(&e))?;
        write_atomic(
            &self.dir.join(format!("{id}.json")),
            info.to_string().as_bytes(),
        )
        .map_err(|e| io(&e))?;
        Ok(att)
    }

    /// Description and the session the attachment is uploaded to.
    fn info(&self, id: i64) -> Result<(Attachment, i64), ChatError> {
        if id <= 0 {
            return Err(ChatError::NotFound);
        }
        let text = fs::read_to_string(self.dir.join(format!("{id}.json"))).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChatError::NotFound
            } else {
                io(&e)
            }
        })?;
        let v: Value = serde_json::from_str(&text).map_err(|_| ChatError::Unavailable)?;
        let att = Attachment {
            id,
            name: v["name"].as_str().unwrap_or("file").to_string(),
            mime: v["mime"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .to_string(),
            size: v["size"].as_u64().unwrap_or(0),
        };
        Ok((att, v["session_id"].as_i64().unwrap_or(0)))
    }

    pub(crate) fn get(&self, id: i64) -> Result<(Attachment, Vec<u8>), ChatError> {
        let (att, _) = self.info(id)?;
        let data = fs::read(self.dir.join(id.to_string())).map_err(|e| io(&e))?;
        Ok((att, data))
    }

    /// This session's attachments by number; foreign and nonexistent ones give `422`.
    pub(crate) fn resolve(
        &self,
        session_id: i64,
        ids: &[i64],
    ) -> Result<Vec<Attachment>, ChatError> {
        ids.iter()
            .map(|id| match self.info(*id) {
                Ok((a, s)) if s == session_id => Ok(a),
                Ok(_) | Err(ChatError::NotFound) => Err(invalid(
                    "attachment_ids",
                    "attachment not found in this session",
                )),
                Err(e) => Err(e),
            })
            .collect()
    }
}
