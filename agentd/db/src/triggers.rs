//! `TriggerStore` (A5-10): triggers in `sessions.db`; deleting a session disables them via a DB trigger (migration 0006).

use std::sync::Arc;

use agentd_core::{PortFuture, Trigger, TriggerError, TriggerResult, TriggerStore};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::{Db, DbError};

const COLS: &str = "id, source, match_json, session_id, enabled";

pub struct TriggerRepo {
    db: Arc<Db>,
}

impl TriggerRepo {
    #[must_use]
    pub fn new(db: Arc<Db>) -> TriggerRepo {
        TriggerRepo { db }
    }
}

/// Key of the "create run" action inside `match_json` (no separate column in the schema, docs/reference/db.md section 3.8).
const TASK_KEY: &str = "task_id";

/// Conditions and action → `match_json`: `task_id` is stored next to the conditions.
fn pack(match_: &Value, task_id: Option<i64>) -> String {
    let mut doc = match_.clone();
    if let (Some(t), Value::Object(m)) = (task_id, &mut doc) {
        m.insert(TASK_KEY.into(), t.into());
    }
    doc.to_string()
}

fn row_of(r: &Row<'_>) -> rusqlite::Result<Trigger> {
    let raw: String = r.get(2)?;
    let mut match_: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let task_id = match &mut match_ {
        Value::Object(m) => m.remove(TASK_KEY).and_then(|v| v.as_i64()),
        _ => None,
    };
    Ok(Trigger {
        id: r.get(0)?,
        source: r.get(1)?,
        match_,
        session_id: r.get(3)?,
        task_id,
        enabled: r.get::<_, i64>(4)? != 0,
    })
}

fn get(c: &Connection, id: i64) -> rusqlite::Result<Option<Trigger>> {
    c.query_row(
        &format!("SELECT {COLS} FROM triggers WHERE id = ?1"),
        [id],
        row_of,
    )
    .optional()
}

fn unavailable(e: &DbError) -> TriggerError {
    tracing::warn!(scope = "triggers", event = "db_unavailable", error = %e, "triggers DB unavailable");
    TriggerError::Unavailable
}

/// Whether the session exists: a trigger for a nonexistent session is neither created nor moved.
fn session_exists(c: &Connection, id: Option<i64>) -> rusqlite::Result<bool> {
    let Some(id) = id else { return Ok(true) };
    Ok(
        c.query_row("SELECT 1 FROM sessions WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
        .is_some(),
    )
}

impl TriggerStore for TriggerRepo {
    fn create(&self, t: Trigger) -> PortFuture<'_, TriggerResult<Trigger>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    if !session_exists(tx, t.session_id)? {
                        return Ok(Err(TriggerError::SessionNotFound));
                    }
                    tx.execute(
                        "INSERT INTO triggers(source, match_json, session_id, enabled) VALUES (?1, ?2, ?3, ?4)",
                        params![t.source, pack(&t.match_, t.task_id), t.session_id, i64::from(t.enabled)],
                    )?;
                    Ok(get(tx, tx.last_insert_rowid())?.ok_or(TriggerError::NotFound))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn get(&self, id: i64) -> PortFuture<'_, TriggerResult<Trigger>> {
        Box::pin(async move {
            self.db
                .read(move |c| get(c, id))
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(TriggerError::NotFound)
        })
    }

    fn list(&self) -> PortFuture<'_, TriggerResult<Vec<Trigger>>> {
        Box::pin(async move {
            self.db
                .read(|c| {
                    c.prepare(&format!("SELECT {COLS} FROM triggers ORDER BY id"))?
                        .query_map([], row_of)?
                        .collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn update(&self, id: i64, t: Trigger) -> PortFuture<'_, TriggerResult<Trigger>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    if get(tx, id)?.is_none() {
                        return Ok(Err(TriggerError::NotFound));
                    }
                    if !session_exists(tx, t.session_id)? {
                        return Ok(Err(TriggerError::SessionNotFound));
                    }
                    tx.execute(
                        "UPDATE triggers SET source = ?1, match_json = ?2, session_id = ?3, enabled = ?4 WHERE id = ?5",
                        params![t.source, pack(&t.match_, t.task_id), t.session_id, i64::from(t.enabled), id],
                    )?;
                    Ok(get(tx, id)?.ok_or(TriggerError::NotFound))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn delete(&self, id: i64) -> PortFuture<'_, TriggerResult<()>> {
        Box::pin(async move {
            let n = self
                .db
                .write(move |tx| tx.execute("DELETE FROM triggers WHERE id = ?1", [id]))
                .await
                .map_err(|e| unavailable(&e))?;
            if n == 0 {
                Err(TriggerError::NotFound)
            } else {
                Ok(())
            }
        })
    }
}
