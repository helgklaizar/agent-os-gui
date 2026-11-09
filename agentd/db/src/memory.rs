//! `MemoryStore` (A6-07): facts in `memory_facts`. The mode comes from the session's effective persona: `own` is a fact with `persona_id`,
//! `shared` (and a session without a persona) is common, `none` is not written.

use std::sync::Arc;

use agentd_core::{CONTEXT_FACTS, MAX_FACTS_PER_MINUTE, MemoryError, MemoryStore, PortFuture};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Db;

pub struct MemoryRepo {
    db: Arc<Db>,
}

impl MemoryRepo {
    #[must_use]
    pub fn new(db: Arc<Db>) -> MemoryRepo {
        MemoryRepo { db }
    }
}

/// `(persona_id, mode)` of the session's effective persona; no persona means shared memory.
fn mode_of(c: &Connection, session_id: i64) -> rusqlite::Result<Option<(Option<i64>, String)>> {
    let Some(session) = crate::tree::session_json(c, session_id)? else {
        return Ok(None);
    };
    let Some(pid) = session["effective_persona_id"].as_i64() else {
        return Ok(Some((None, "shared".to_string())));
    };
    let mode: Option<String> = c
        .query_row(
            "SELECT mem_mode FROM personas WHERE id = ?1 AND deleted_at IS NULL",
            [pid],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Some(match mode {
        Some(m) => (Some(pid), m),
        None => (None, "shared".to_string()),
    }))
}

/// Facts for the reply context: own and shared for `own`, shared for `shared`, nothing for `none`; newest first.
pub(crate) fn context_facts(c: &Connection, session_id: i64) -> rusqlite::Result<Vec<String>> {
    let Some((pid, mode)) = mode_of(c, session_id)? else {
        return Ok(Vec::new());
    };
    let sql = match (mode.as_str(), pid) {
        ("own", Some(_)) => {
            "SELECT text FROM memory_facts WHERE persona_id = ?1 OR persona_id IS NULL ORDER BY id DESC LIMIT ?2"
        }
        ("none", _) => return Ok(Vec::new()),
        _ => {
            "SELECT text FROM memory_facts WHERE persona_id IS NULL AND ?1 IS ?1 ORDER BY id DESC LIMIT ?2"
        }
    };
    c.prepare(sql)?
        .query_map(params![pid, CONTEXT_FACTS], |r| r.get(0))?
        .collect()
}

impl MemoryStore for MemoryRepo {
    fn add(
        &self,
        session_id: i64,
        text: String,
        now: i64,
    ) -> PortFuture<'_, Result<i64, MemoryError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let Some((pid, mode)) = mode_of(tx, session_id)? else {
                        return Ok(Err(MemoryError::Invalid));
                    };
                    if mode == "none" {
                        return Ok(Err(MemoryError::Disabled));
                    }
                    let recent: i64 = tx.query_row(
                        "SELECT count(*) FROM memory_facts WHERE session_id = ?1 AND ts > ?2",
                        params![session_id, now - 60],
                        |r| r.get(0),
                    )?;
                    if recent >= MAX_FACTS_PER_MINUTE {
                        return Ok(Err(MemoryError::RateLimited));
                    }
                    let owner = if mode == "own" { pid } else { None };
                    tx.execute(
                        "INSERT INTO memory_facts(persona_id, session_id, text, source, ts) VALUES (?1, ?2, ?3, 'agent', ?4)",
                        params![owner, session_id, text, now],
                    )?;
                    Ok(Ok(tx.last_insert_rowid()))
                })
                .await
                .map_err(|_| MemoryError::Unavailable)?
        })
    }
}
