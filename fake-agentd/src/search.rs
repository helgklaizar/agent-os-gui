//! Unified search `GET /search` and the `index.db` model (api.md).
//!
//! `index.db` is a separate file next to `sessions.db`, the GUI does not write it. The real `agent-index` (SQLite + `sqlite-vec`) is
//! simplified here: a small fixture of documents with ready vectors (`f32` BLOB), no vector search is performed - a case-insensitive
//! substring match. The `mcp_memory` source is emulated by the `memory_mcp` fixture. The core returns text as is: the view escapes.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::db::Db;
use crate::store::{StoreError, db_err};

/// Memory source (`settings.memory.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySource {
    Local,
    McpMemory,
    Both,
}

impl MemorySource {
    pub fn as_str(self) -> &'static str {
        match self {
            MemorySource::Local => "local",
            MemorySource::McpMemory => "mcp_memory",
            MemorySource::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Option<MemorySource> {
        [
            MemorySource::Local,
            MemorySource::McpMemory,
            MemorySource::Both,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

/// `index.db` documents: `(origin, id, title, body, vector)`. Vectors are preset (in the real index `agent-index` computes them).
const FIXTURE: &[(&str, &str, &str, &str, [f32; 4])] = &[
    (
        "file",
        "notes/roadmap.md",
        "Roadmap",
        "release 0.1 roadmap: bar, search, settings",
        [0.1, 0.2, 0.3, 0.4],
    ),
    (
        "file",
        "docs/backup.md",
        "Backup policy",
        "backup daily, restore check",
        [0.4, 0.3, 0.2, 0.1],
    ),
    (
        "memory_mcp",
        "memory_mcp-1",
        "Deploy via CI",
        "fact from memory_mcp: deploy is performed by the release script",
        [0.9, 0.1, 0.0, 0.2],
    ),
];

const SNIPPET_CHARS: usize = 160;

pub struct SearchIndex {
    path: PathBuf,
}

fn snippet(text: &str) -> String {
    text.chars().take(SNIPPET_CHARS).collect()
}

fn matches(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(needle)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool, StoreError> {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    Ok(n == 1)
}

fn hit(kind: &str, source: &str, title: &str, text: &str, r: Value) -> Value {
    json!({ "kind": kind, "source": source, "title": title, "snippet": snippet(text), "ref": r })
}

impl SearchIndex {
    /// Creates `index.db` in `dir` and fills it with the fixture if the file is empty.
    pub fn open(dir: &Path) -> Result<SearchIndex, StoreError> {
        let path = dir.join("index.db");
        let conn = Connection::open(&path).map_err(db_err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS docs (
               id TEXT PRIMARY KEY, origin TEXT NOT NULL CHECK (origin IN ('file','memory_mcp')),
               title TEXT NOT NULL, body TEXT NOT NULL, embedding BLOB NOT NULL);",
        )
        .map_err(db_err)?;
        for (origin, id, title, body, vec) in FIXTURE {
            let blob: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();
            conn.execute(
                "INSERT OR IGNORE INTO docs(id, origin, title, body, embedding) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, origin, title, body, blob],
            )
            .map_err(db_err)?;
        }
        Ok(SearchIndex { path })
    }

    fn docs(
        &self,
        origin: &str,
        needle: &str,
    ) -> Result<Vec<(String, String, String)>, StoreError> {
        let conn =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(db_err)?;
        let mut stmt = conn
            .prepare("SELECT id, title, body FROM docs WHERE origin = ?1 ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([origin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_err)?;
        let all: Vec<(String, String, String)> = rows.collect::<Result<_, _>>().map_err(db_err)?;
        Ok(all
            .into_iter()
            .filter(|(_, t, b)| matches(t, needle) || matches(b, needle))
            .collect())
    }

    /// Results by group in the order: sessions, messages, mail, files, memory. An empty query gives an empty answer.
    pub fn search(
        &self,
        db: &Db,
        source: MemorySource,
        q: &str,
        limit: usize,
    ) -> Result<Vec<Value>, StoreError> {
        let needle = q.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let conn = db.reader().map_err(db_err)?;
        let mut out: Vec<Value> = Vec::new();

        if has_table(&conn, "sessions")? {
            let mut stmt = conn
                .prepare("SELECT id, name FROM sessions WHERE archived = 0 ORDER BY id")
                .map_err(db_err)?;
            let rows: Vec<(i64, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(db_err)?
                .collect::<Result<_, _>>()
                .map_err(db_err)?;
            for (id, name) in rows.into_iter().filter(|(_, n)| matches(n, &needle)) {
                out.push(hit(
                    "session",
                    "local",
                    &name,
                    "",
                    json!({ "session_id": id }),
                ));
            }
        }
        if has_table(&conn, "messages")? {
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.session_id, m.body_md, s.name FROM messages m JOIN sessions s ON s.id = m.session_id ORDER BY m.id",
                )
                .map_err(db_err)?;
            let rows: Vec<(i64, i64, String, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(db_err)?
                .collect::<Result<_, _>>()
                .map_err(db_err)?;
            for (id, sid, body, name) in rows.into_iter().filter(|(_, _, b, _)| matches(b, &needle))
            {
                out.push(hit(
                    "message",
                    "local",
                    &name,
                    &body,
                    json!({ "session_id": sid, "message_id": id }),
                ));
            }
        }
        if has_table(&conn, "gw_messages")? && has_table(&conn, "accounts")? {
            let mut stmt = conn
                .prepare(
                    "SELECT g.id, g.body, a.type, a.label FROM gw_messages g JOIN accounts a ON a.id = g.account_id ORDER BY g.id",
                )
                .map_err(db_err)?;
            let rows: Vec<(i64, String, String, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(db_err)?
                .collect::<Result<_, _>>()
                .map_err(db_err)?;
            for (id, body, ty, label) in rows.into_iter().filter(|(_, b, _, _)| matches(b, &needle))
            {
                let kind = if ty == "gmail" || ty == "mail" {
                    "mail"
                } else {
                    "message"
                };
                out.push(hit(kind, "local", &label, &body, json!({ "id": id })));
            }
        }
        for (id, title, body) in self.docs("file", &needle)? {
            out.push(hit("file", "local", &title, &body, json!({ "path": id })));
        }
        if matches!(source, MemorySource::Local | MemorySource::Both)
            && has_table(&conn, "memory_facts")?
        {
            let mut stmt = conn
                .prepare("SELECT id, text FROM memory_facts ORDER BY id")
                .map_err(db_err)?;
            let rows: Vec<(i64, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(db_err)?
                .collect::<Result<_, _>>()
                .map_err(db_err)?;
            for (id, text) in rows.into_iter().filter(|(_, t)| matches(t, &needle)) {
                out.push(hit(
                    "memory",
                    "local",
                    &snippet(&text),
                    &text,
                    json!({ "id": id.to_string() }),
                ));
            }
        }
        if matches!(source, MemorySource::McpMemory | MemorySource::Both) {
            for (id, title, body) in self.docs("memory_mcp", &needle)? {
                out.push(hit(
                    "memory",
                    "mcp_memory",
                    &title,
                    &body,
                    json!({ "id": id }),
                ));
            }
        }
        out.truncate(limit);
        Ok(out)
    }
}
