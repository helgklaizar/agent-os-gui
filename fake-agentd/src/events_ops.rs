//! Reading and marking events: JSON `Event`, list with `counts`, bulk "read", `events` topic frames.

use rusqlite::{Connection, Row, params_from_iter, types::Value as Sql};
use serde_json::{Map, Value, json};

use crate::FrameType;
use crate::events_db::{self, NewEvent};
use crate::state::Shared;

const COLS: &str = "id, kind, sub, level, text, session_id, source, unread, ts";

/// `Event` from api.md section 6: `tab` is computed by the core (default `chat`), `dedup_key` is not exposed.
fn from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "sub": r.get::<_, String>(2)?,
        "level": r.get::<_, String>(3)?, "text": r.get::<_, String>(4)?,
        "session_id": r.get::<_, Option<i64>>(5)?, "tab": "chat", "source": r.get::<_, Option<String>>(6)?,
        "unread": r.get::<_, i64>(7)? != 0, "ts": r.get::<_, i64>(8)?,
    }))
}

/// Selection filter. `unread`, `before` and `limit` apply to the page and do not affect `counts`.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub kind: Option<String>,
    pub sub: Option<String>,
    pub session_id: Option<i64>,
    pub ids: Option<Vec<i64>>,
    pub unread: Option<bool>,
    pub before: Option<i64>,
    pub limit: Option<i64>,
}

fn selection(f: &Filter) -> (String, Vec<Sql>) {
    let mut sql = String::from(" WHERE 1 = 1");
    let mut args: Vec<Sql> = Vec::new();
    if let Some(k) = &f.kind {
        sql.push_str(" AND kind = ?");
        args.push(Sql::Text(k.clone()));
    }
    if let Some(s) = &f.sub {
        sql.push_str(" AND sub = ?");
        args.push(Sql::Text(s.clone()));
    }
    if let Some(s) = f.session_id {
        sql.push_str(" AND session_id = ?");
        args.push(Sql::Integer(s));
    }
    if let Some(ids) = &f.ids {
        let marks = vec!["?"; ids.len()].join(", ");
        sql.push_str(&format!(" AND id IN ({marks})"));
        args.extend(ids.iter().map(|i| Sql::Integer(*i)));
    }
    (sql, args)
}

/// Unread for the `kind/sub/session_id` selection: `total`, `by_kind`, `by_sub` (key `kind.sub`).
pub fn counts(conn: &Connection, f: &Filter) -> rusqlite::Result<Value> {
    let (wh, args) = selection(f);
    let sql = format!(
        "SELECT kind, sub, count(*) FROM events{wh} AND unread = 1 GROUP BY kind, sub ORDER BY kind, sub"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(args), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let (mut total, mut by_kind, mut by_sub) = (0, Map::new(), Map::new());
    for row in rows {
        let (kind, sub, n) = row?;
        total += n;
        let have = by_kind.get(&kind).and_then(Value::as_i64).unwrap_or(0);
        by_kind.insert(kind.clone(), json!(have + n));
        by_sub.insert(format!("{kind}.{sub}"), json!(n));
    }
    Ok(json!({ "total": total, "by_kind": by_kind, "by_sub": by_sub }))
}

/// Page of events (newest first, pagination by `id`) and `counts` over the whole selection.
pub fn list(conn: &Connection, f: &Filter) -> rusqlite::Result<Value> {
    let (mut wh, mut args) = selection(f);
    if let Some(u) = f.unread {
        wh.push_str(" AND unread = ?");
        args.push(Sql::Integer(i64::from(u)));
    }
    if let Some(b) = f.before {
        wh.push_str(" AND id < ?");
        args.push(Sql::Integer(b));
    }
    args.push(Sql::Integer(f.limit.unwrap_or(50)));
    let sql = format!("SELECT {COLS} FROM events{wh} ORDER BY id DESC LIMIT ?");
    let mut st = conn.prepare(&sql)?;
    let items = st
        .query_map(params_from_iter(args), from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({ "items": items, "counts": counts(conn, f)? }))
}

pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        &format!("SELECT {COLS} FROM events WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
}

/// How many latest events go into the `events` topic snapshot (the GUI keeps as many; the rest is `GET /events?before=`).
const SNAPSHOT_LIMIT: i64 = 200;

/// First frame of the `events` topic: latest events and `counts` (same as `GET /events`).
pub fn snapshot(shared: &Shared) -> Value {
    let read = || -> Result<Value, String> {
        let conn = shared.db.reader().map_err(|e| e.to_string())?;
        let last = Filter {
            limit: Some(SNAPSHOT_LIMIT),
            ..Filter::default()
        };
        list(&conn, &last).map_err(|e| e.to_string())
    };
    read().unwrap_or_else(|e| {
        tracing::error!(error = %e, "events snapshot not read");
        json!({ "items": [], "counts": { "total": 0, "by_kind": {}, "by_sub": {} } })
    })
}

/// Inserts an event and sends an `event` frame. `None` - repeated `dedup_key`, no event and no frame.
pub fn add(shared: &Shared, e: &NewEvent<'_>) -> Result<Option<i64>, crate::DbError> {
    let (id, event) = shared.db.write(|w| {
        let Some(id) = events_db::insert(w, e)? else {
            return Ok((None, None));
        };
        Ok((Some(id), get(w, id)?))
    })?;
    if let Some(event) = event {
        shared.hub.emit_keep("events", FrameType::Event, event);
    }
    Ok(id)
}

/// Marks the selection's unread events as read; a `{read: [id...]}` frame only if something changed.
pub fn mark_read(shared: &Shared, f: &Filter) -> Result<Vec<i64>, crate::DbError> {
    let ids = shared.db.write(|w| {
        let tx = w.transaction()?;
        let (wh, args) = selection(f);
        let sql = format!("SELECT id FROM events{wh} AND unread = 1 ORDER BY id");
        let ids = tx
            .prepare(&sql)?
            .query_map(params_from_iter(args), |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in &ids {
            tx.execute("UPDATE events SET unread = 0 WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(ids)
    })?;
    if !ids.is_empty() {
        shared
            .hub
            .emit_keep("events", FrameType::Delta, json!({ "read": ids }));
    }
    Ok(ids)
}
