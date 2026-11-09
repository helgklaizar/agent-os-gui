//! Message SQL: insert, pages, append, close, cleanup, `streaming` → `interrupted` transition.

use agentd_core::{Attachment, ChatError, FieldError, Posted};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde_json::{Value, json};

const COLS: &str = "id, session_id, client_id, role, author, body_md, status, meta_json, ts";

pub(crate) fn message_from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    let meta: Option<String> = r.get(7)?;
    let meta = meta
        .and_then(|m| serde_json::from_str::<Value>(&m).ok())
        .unwrap_or(Value::Null);
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "session_id": r.get::<_, i64>(1)?,
        "client_id": r.get::<_, Option<String>>(2)?,
        "role": r.get::<_, String>(3)?,
        "author": r.get::<_, Option<String>>(4)?,
        "body_md": r.get::<_, String>(5)?,
        "status": r.get::<_, String>(6)?,
        "meta": meta,
        "ts": r.get::<_, i64>(8)?,
    }))
}

pub(crate) fn session_exists(c: &Connection, id: i64) -> rusqlite::Result<bool> {
    c.query_row("SELECT 1 FROM sessions WHERE id = ?1", [id], |_| Ok(()))
        .optional()
        .map(|o| o.is_some())
}

pub(crate) fn message(c: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        &format!("SELECT {COLS} FROM messages WHERE id = ?1"),
        [id],
        message_from_row,
    )
    .optional()
}

/// Page from newest to oldest.
pub(crate) fn page(
    c: &Connection,
    session_id: i64,
    before: Option<i64>,
    limit: i64,
) -> rusqlite::Result<Vec<Value>> {
    c.prepare(&format!(
        "SELECT {COLS} FROM messages WHERE session_id = ?1 AND (?2 IS NULL OR id < ?2) ORDER BY id DESC LIMIT ?3"
    ))?
    .query_map(params![session_id, before, limit], message_from_row)?
    .collect()
}

pub(crate) fn post_user(
    tx: &Transaction<'_>,
    now: i64,
    session_id: i64,
    client_id: &str,
    body: &str,
    attachments: &[Attachment],
) -> rusqlite::Result<Result<Posted, ChatError>> {
    if !session_exists(tx, session_id)? {
        return Ok(Err(ChatError::NotFound));
    }
    let old: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, body_md FROM messages WHERE session_id = ?1 AND client_id = ?2",
            params![session_id, client_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, text)) = old {
        if text != body {
            return Ok(Err(ChatError::Conflict("client_id_conflict")));
        }
        return Ok(Ok(Posted::Existing(message(tx, id)?.unwrap_or_default())));
    }
    let meta = (!attachments.is_empty()).then(|| {
        json!({"attachments": attachments.iter().map(Attachment::to_json).collect::<Vec<_>>()})
            .to_string()
    });
    tx.execute(
        "INSERT INTO messages(session_id, client_id, role, body_md, status, meta_json, ts)
         VALUES (?1, ?2, 'user', ?3, 'done', ?4, ?5)",
        params![session_id, client_id, body, meta, now],
    )?;
    let id = tx.last_insert_rowid();
    Ok(Ok(Posted::Created(message(tx, id)?.unwrap_or_default())))
}

pub(crate) fn create_agent(
    tx: &Transaction<'_>,
    now: i64,
    session_id: i64,
    author: &str,
) -> rusqlite::Result<Result<Value, ChatError>> {
    if !session_exists(tx, session_id)? {
        return Ok(Err(ChatError::NotFound));
    }
    tx.execute(
        "INSERT INTO messages(session_id, role, author, body_md, status, ts) VALUES (?1, 'agent', ?2, '', 'streaming', ?3)",
        params![session_id, author, now],
    )?;
    Ok(Ok(message(tx, tx.last_insert_rowid())?.unwrap_or_default()))
}

/// A `role = system` message (permission request card).
pub(crate) fn create_system(
    tx: &Transaction<'_>,
    now: i64,
    session_id: i64,
    body: &str,
    meta: &Value,
) -> rusqlite::Result<Result<Value, ChatError>> {
    if !session_exists(tx, session_id)? {
        return Ok(Err(ChatError::NotFound));
    }
    tx.execute(
        "INSERT INTO messages(session_id, role, author, body_md, status, meta_json, ts)
         VALUES (?1, 'system', 'Permissions', ?2, 'done', ?3, ?4)",
        params![session_id, body, meta.to_string(), now],
    )?;
    Ok(Ok(message(tx, tx.last_insert_rowid())?.unwrap_or_default()))
}

/// A `role = tool` message: the result of an agent action.
pub(crate) fn create_tool(
    tx: &Transaction<'_>,
    now: i64,
    session_id: i64,
    body: &str,
    meta: &Value,
) -> rusqlite::Result<Result<Value, ChatError>> {
    if !session_exists(tx, session_id)? {
        return Ok(Err(ChatError::NotFound));
    }
    tx.execute(
        "INSERT INTO messages(session_id, role, author, body_md, status, meta_json, ts)
         VALUES (?1, 'tool', 'Tool', ?2, 'done', ?3, ?4)",
        params![session_id, body, meta.to_string(), now],
    )?;
    Ok(Ok(message(tx, tx.last_insert_rowid())?.unwrap_or_default()))
}

/// The `patch` fields replace same-named fields of the `meta` of the request card `request_id`.
pub(crate) fn update_card(
    tx: &Transaction<'_>,
    request_id: i64,
    patch: &Value,
) -> rusqlite::Result<Option<Value>> {
    let row: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, meta_json FROM messages WHERE role = 'system'
             AND json_extract(meta_json, '$.kind') = 'permission_request'
             AND json_extract(meta_json, '$.request_id') = ?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, meta)) = row else {
        return Ok(None);
    };
    let mut meta: Value = serde_json::from_str(&meta).unwrap_or(Value::Null);
    if let (Value::Object(m), Value::Object(p)) = (&mut meta, patch) {
        for (k, v) in p {
            m.insert(k.clone(), v.clone());
        }
    }
    tx.execute(
        "UPDATE messages SET meta_json = ?2 WHERE id = ?1",
        params![id, meta.to_string()],
    )?;
    message(tx, id)
}

fn body_len(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Option<usize>> {
    let n: Option<i64> = tx
        .query_row(
            "SELECT length(CAST(body_md AS BLOB)) FROM messages WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(n.map(|n| usize::try_from(n).unwrap_or(0)))
}

/// Appends text only to a message with status `streaming`; `None` means no row or it is already closed.
pub(crate) fn append(tx: &Transaction<'_>, id: i64, text: &str) -> rusqlite::Result<Option<usize>> {
    let n = tx.execute(
        "UPDATE messages SET body_md = body_md || ?2 WHERE id = ?1 AND status = 'streaming'",
        params![id, text],
    )?;
    if n == 0 {
        return Ok(None);
    }
    body_len(tx, id)
}

pub(crate) fn finish(
    tx: &Transaction<'_>,
    id: i64,
    status: &str,
    meta: Option<&Value>,
) -> rusqlite::Result<Option<usize>> {
    let n = tx.execute(
        "UPDATE messages SET status = ?2, meta_json = COALESCE(?3, meta_json) WHERE id = ?1 AND status = 'streaming'",
        params![id, status, meta.map(Value::to_string)],
    )?;
    if n == 0 {
        return Ok(None);
    }
    body_len(tx, id)
}

pub(crate) fn clear(
    tx: &Transaction<'_>,
    session_id: i64,
) -> rusqlite::Result<Result<(), ChatError>> {
    if !session_exists(tx, session_id)? {
        return Ok(Err(ChatError::NotFound));
    }
    tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
    Ok(Ok(()))
}

pub(crate) fn interrupt_streaming(tx: &Transaction<'_>) -> rusqlite::Result<u64> {
    let n = tx.execute(
        "UPDATE messages SET status = 'interrupted' WHERE status = 'streaming'",
        [],
    )?;
    Ok(n as u64)
}

pub(crate) fn invalid(field: &str, msg: &str) -> ChatError {
    ChatError::Invalid(vec![FieldError::new(field, msg)])
}
