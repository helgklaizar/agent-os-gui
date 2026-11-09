//! `index.db`: a separate index file (`0600`) with its own schema; `sessions.db` is neither opened nor written here.
//! FTS5 with external content: the `docs` table holds the records, `docs_fts` is the index over `title` and `body`.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use agentd_core::SearchHit;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

const SCHEMA_V1: &str = "
CREATE TABLE docs (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('session','message','file','memory')),
  key        TEXT NOT NULL,
  session_id INTEGER,
  title      TEXT NOT NULL,
  body       TEXT NOT NULL,
  rev        TEXT,
  UNIQUE (kind, key)
);
CREATE INDEX idx_docs_session ON docs(session_id);
CREATE VIRTUAL TABLE docs_fts USING fts5(title, body, content='docs', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER docs_ai AFTER INSERT ON docs BEGIN
  INSERT INTO docs_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER docs_ad AFTER DELETE ON docs BEGIN
  INSERT INTO docs_fts(docs_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TRIGGER docs_au AFTER UPDATE ON docs BEGIN
  INSERT INTO docs_fts(docs_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
  INSERT INTO docs_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TABLE state (k TEXT PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO state(k, v) VALUES ('schema', '1');
";

/// How many query words are counted.
const MAX_TOKENS: usize = 10;

pub(super) fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        drop(std::fs::create_dir_all(dir));
    }
    // The file is created with `0600` right away: no window with wide permissions.
    drop(
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(path),
    );
    let c = Connection::open(path)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    // The index schema version is a record in `state`; `user_version` belongs only to the `sessions.db` migration runner.
    let has: i64 = c.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'docs'",
        [],
        |r| r.get(0),
    )?;
    if has == 0 {
        c.execute_batch(SCHEMA_V1)?;
    }
    Ok(c)
}

/// Query words: only letters and digits; each is quoted with a prefix, so FTS5 operators (`OR`, `NOT`,
/// `NEAR`, `col:`, `*`, `-`, `^`) become plain words and are not executed. Empty gives `None`.
pub(super) fn match_expr(q: &str) -> Option<String> {
    let tokens: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(MAX_TOKENS)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!tokens.is_empty()).then(|| tokens.join(" "))
}

pub(super) fn upsert(
    c: &Connection,
    kind: &str,
    key: &str,
    session_id: Option<i64>,
    title: &str,
    body: &str,
    rev: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO docs(kind, key, session_id, title, body, rev) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, key) DO UPDATE SET session_id = ?3, title = ?4, body = ?5, rev = ?6
         WHERE title <> ?4 OR body <> ?5 OR rev IS NOT ?6 OR session_id IS NOT ?3",
        params![kind, key, session_id, title, body, rev],
    )?;
    Ok(())
}

pub(super) fn state_get(c: &Connection, k: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT v FROM state WHERE k = ?1", [k], |r| r.get(0))
        .optional()
}

pub(super) fn state_set(c: &Connection, k: &str, v: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO state(k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
        params![k, v],
    )?;
    Ok(())
}

pub(super) fn query(c: &Connection, q: &str, limit: usize) -> rusqlite::Result<Vec<SearchHit>> {
    let Some(expr) = match_expr(q) else {
        return Ok(Vec::new());
    };
    let mut stmt = c.prepare(
        "SELECT d.kind, d.key, d.session_id, d.title, snippet(docs_fts, -1, '', '', '…', 12)
         FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
         WHERE docs_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![expr, i64::try_from(limit).unwrap_or(50)], |r| {
        let (kind, key, sid, title, snippet): (String, String, Option<i64>, String, String) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        let reference = match kind.as_str() {
            "session" => json!({"session_id": sid}),
            "message" => json!({"session_id": sid, "message_id": key.parse::<i64>().ok()}),
            _ => json!({"path": key}),
        };
        Ok(SearchHit {
            kind,
            source: "local",
            title,
            snippet,
            reference,
        })
    })?;
    rows.collect()
}
