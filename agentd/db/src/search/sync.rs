//! Syncing the index with the sources: sessions and messages from `sessions.db` (read), `AGENT.md` and `memory/` files.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;

use super::{SearchRepo, index};

/// Messages per read pass and passes per query.
const BATCH: i64 = 500;
const MAX_BATCHES: usize = 20;
/// Larger files are not indexed; allowed extensions.
const MAX_FILE_BYTES: u64 = 256 * 1024;
const EXTENSIONS: [&str; 2] = ["md", "txt"];
const MAX_FILES: usize = 2000;

struct Fresh {
    sessions: Vec<(i64, String)>,
    messages: Vec<(i64, i64, String)>,
    watermark: i64,
}

fn read_fresh(c: &Connection, watermark: i64) -> rusqlite::Result<Fresh> {
    let sessions = c
        .prepare("SELECT id, name FROM sessions")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // An unfinished message is not skipped: the watermark is the smallest id among `streaming`.
    let barrier: i64 = c.query_row(
        "SELECT COALESCE(MIN(id), 9223372036854775807) FROM messages WHERE status = 'streaming'",
        [],
        |r| r.get(0),
    )?;
    let messages = c
        .prepare(
            "SELECT id, session_id, body_md FROM messages
             WHERE id > ?1 AND id < ?2 AND status = 'done' AND role IN ('user','agent') ORDER BY id LIMIT ?3",
        )?
        .query_map(rusqlite::params![watermark, barrier, BATCH], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Next watermark: if the batch is full, by its last message, otherwise up to the unfinished boundary.
    let next = if messages.len() as i64 == BATCH {
        messages.last().map_or(watermark, |m| m.0)
    } else {
        // Messages with an id below the barrier but not `done` (stopped, error) are passed too.
        c.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM messages WHERE id < ?1",
            [barrier],
            |r| r.get(0),
        )?
    };
    Ok(Fresh {
        sessions,
        messages,
        watermark: next.max(watermark),
    })
}

fn apply_sessions(c: &Connection, sessions: &[(i64, String)]) -> rusqlite::Result<()> {
    let live: HashSet<i64> = sessions.iter().map(|s| s.0).collect();
    let known: Vec<i64> = c
        .prepare("SELECT DISTINCT session_id FROM docs WHERE session_id IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for gone in known.into_iter().filter(|id| !live.contains(id)) {
        c.execute("DELETE FROM docs WHERE session_id = ?1", [gone])?;
    }
    for (id, name) in sessions {
        index::upsert(c, "session", &id.to_string(), Some(*id), name, "", None)?;
        // Rename: message titles follow the session name.
        c.execute(
            "UPDATE docs SET title = ?1 WHERE kind = 'message' AND session_id = ?2 AND title <> ?1",
            rusqlite::params![name, id],
        )?;
    }
    Ok(())
}

fn rel(root: &Path, p: &Path) -> Option<String> {
    Some(p.strip_prefix(root).ok()?.to_str()?.replace('\\', "/"))
}

/// Source files: `AGENT.md` and regular `memory/` files (links are not dereferenced).
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String, String)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else {
            continue;
        };
        if md.is_dir() {
            collect(root, &p, out);
        } else if md.is_file() && md.len() <= MAX_FILE_BYTES && ext_ok(&p) {
            push_file(root, &p, &md, out);
        }
    }
}

fn ext_ok(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn push_file(root: &Path, p: &Path, md: &fs::Metadata, out: &mut Vec<(String, String, String)>) {
    let (Some(key), Ok(text)) = (rel(root, p), fs::read_to_string(p)) else {
        return;
    };
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    out.push((key, text, format!("{mtime}:{}", md.len())));
}

fn apply_files(c: &Connection, root: &Path) -> rusqlite::Result<()> {
    let mut files = Vec::new();
    let agent = root.join("AGENT.md");
    if let Ok(md) = fs::symlink_metadata(&agent)
        && md.is_file()
        && md.len() <= MAX_FILE_BYTES
    {
        push_file(root, &agent, &md, &mut files);
    }
    collect(root, &root.join("memory"), &mut files);
    let seen: HashSet<&str> = files.iter().map(|f| f.0.as_str()).collect();
    let known: Vec<String> = c
        .prepare("SELECT key FROM docs WHERE kind = 'file'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for gone in known.iter().filter(|k| !seen.contains(k.as_str())) {
        c.execute("DELETE FROM docs WHERE kind = 'file' AND key = ?1", [gone])?;
    }
    for (key, text, rev) in &files {
        index::upsert(c, "file", key, None, key, text, Some(rev))?;
    }
    Ok(())
}

impl SearchRepo {
    /// Brings the index to the current state of the sources; `Err(())` means the DB or the index is unavailable.
    pub(super) async fn sync(&self) -> Result<(), ()> {
        for _ in 0..MAX_BATCHES {
            let wm = {
                let conn = self.conn.clone();
                tokio::task::spawn_blocking(move || {
                    let c = conn
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    index::state_get(&c, "messages_wm")
                })
                .await
                .map_err(|_| ())?
                .map_err(|_| ())?
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0)
            };
            let fresh = self
                .db
                .read(move |c| read_fresh(c, wm))
                .await
                .map_err(|_| ())?;
            let more = fresh.messages.len() as i64 == BATCH;
            let (conn, root) = (self.conn.clone(), self.root.clone());
            tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
                let mut c = conn
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let tx = c.transaction()?;
                apply_sessions(&tx, &fresh.sessions)?;
                let names: std::collections::HashMap<i64, &str> =
                    fresh.sessions.iter().map(|s| (s.0, s.1.as_str())).collect();
                for (id, sid, body) in &fresh.messages {
                    if let Some(title) = names.get(sid) {
                        index::upsert(
                            &tx,
                            "message",
                            &id.to_string(),
                            Some(*sid),
                            title,
                            body,
                            None,
                        )?;
                    }
                }
                apply_files(&tx, &root)?;
                index::state_set(&tx, "messages_wm", &fresh.watermark.to_string())?;
                tx.commit()
            })
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
            if !more {
                return Ok(());
            }
        }
        Ok(())
    }
}
