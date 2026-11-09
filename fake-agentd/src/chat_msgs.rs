//! Chat messages: `messages` rows, JSON `Message`, snapshot and frames of the `session.<id>` topic.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

use crate::FrameType;
use crate::state::Shared;
use crate::tree;

const COLS: &str = "id, session_id, client_id, role, author, body_md, status, meta_json, ts";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    let meta: Option<String> = r.get(7)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "session_id": r.get::<_, i64>(1)?, "client_id": r.get::<_, Option<String>>(2)?,
        "role": r.get::<_, String>(3)?, "author": r.get::<_, Option<String>>(4)?, "body_md": r.get::<_, String>(5)?,
        "status": r.get::<_, String>(6)?,
        "meta": meta.and_then(|m| serde_json::from_str::<Value>(&m).ok()),
        "ts": r.get::<_, i64>(8)?,
    }))
}

pub struct NewMsg<'a> {
    pub session: i64,
    pub client_id: Option<&'a str>,
    pub role: &'a str,
    pub author: Option<&'a str>,
    pub body: &'a str,
    pub status: &'a str,
    pub meta: Option<&'a Value>,
}

pub fn insert(conn: &Connection, now: i64, m: &NewMsg<'_>) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO messages(session_id, client_id, role, author, body_md, status, meta_json, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![m.session, m.client_id, m.role, m.author, m.body, m.status, m.meta.map(Value::to_string), now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM messages WHERE id = ?1"),
        params![id],
        from_row,
    )
    .optional()
}

pub fn by_client(conn: &Connection, session: i64, client: &str) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM messages WHERE session_id = ?1 AND client_id = ?2"),
        params![session, client],
        from_row,
    )
    .optional()
}

/// History page from newest to oldest.
pub fn page(
    conn: &Connection,
    session: i64,
    before: Option<i64>,
    limit: i64,
) -> rusqlite::Result<Vec<Value>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM messages WHERE session_id = ?1 AND (?2 IS NULL OR id < ?2) ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = st.query_map(params![session, before, limit], from_row)?;
    rows.collect()
}

pub fn append(conn: &Connection, id: i64, text: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET body_md = body_md || ?1 WHERE id = ?2",
        params![text, id],
    )?;
    Ok(())
}

pub fn set_status(conn: &Connection, id: i64, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn set_meta(conn: &Connection, id: i64, meta: &Value) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET meta_json = ?1 WHERE id = ?2",
        params![meta.to_string(), id],
    )?;
    Ok(())
}

/// Snapshot of `session.<id>` (api.md section 4.4): messages of the last page in ascending `id`, open requests, tasks.
pub fn snapshot(shared: &Shared, session: i64) -> Value {
    let Ok(conn) = shared.db.reader() else {
        return json!({});
    };
    let Ok(t) = tree::load(&conn) else {
        return json!({});
    };
    let Some(s) = t.session(session) else {
        return json!({ "session": null, "messages": [], "open_requests": [], "tasks": [] });
    };
    let mut messages = page(&conn, session, None, 50).unwrap_or_default();
    messages.reverse();
    json!({
        "session": t.session_json(s), "messages": messages,
        "open_requests": shared.chat.open_for(session), "tasks": [],
    })
}

/// Topic event: `data.kind` and fields.
pub fn emit(shared: &Shared, session: i64, kind: &str, mut fields: Value) {
    fields["kind"] = Value::String(kind.to_string());
    shared
        .hub
        .emit_keep(&format!("session.{session}"), FrameType::Event, fields);
}

pub fn emit_created(shared: &Shared, message: &Value) {
    let session = message["session_id"].as_i64().unwrap_or(0);
    emit(
        shared,
        session,
        "message.created",
        json!({ "message": message }),
    );
}

/// "Agent · Persona" (the session's effective persona) or "Agent".
pub fn author(conn: &Connection, session: i64) -> String {
    let name = tree::effective_persona_id(conn, session)
        .ok()
        .flatten()
        .and_then(|p| {
            conn.query_row("SELECT name FROM personas WHERE id = ?1", params![p], |r| {
                r.get::<_, String>(0)
            })
            .ok()
        });
    name.map_or_else(|| "Agent".to_string(), |n| format!("Agent · {n}"))
}

/// Insert with a `message.created` frame; `None` if there is no session or on a DB failure.
pub fn post(shared: &Shared, m: &NewMsg<'_>) -> Option<Value> {
    let now = shared.clock.now_unix();
    let msg = shared
        .db
        .write(|w| insert(w, now, m).and_then(|id| get(w, id)))
        .ok()
        .flatten()?;
    emit_created(shared, &msg);
    Some(msg)
}

/// At core start: `streaming` rows after a crash become `interrupted`.
pub fn interrupt_streaming(shared: &Shared) -> Result<(), crate::db::DbError> {
    shared.db.write(|w| {
        w.execute(
            "UPDATE messages SET status = 'interrupted' WHERE status = 'streaming'",
            [],
        )
    })?;
    Ok(())
}
