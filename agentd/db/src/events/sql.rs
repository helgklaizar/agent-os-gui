//! Feed SQL: insert with deduplication, selection with counters, marking as read.

use agentd_core::{Event, EventFilter, NewEvent, ReadSelector};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use serde_json::{Map, Value, json};

const COLS: &str = "id, kind, sub, level, text, session_id, source, unread, ts";

fn event_of(r: &Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        kind: r.get(1)?,
        sub: r.get(2)?,
        level: r.get(3)?,
        text: r.get(4)?,
        session_id: r.get(5)?,
        source: r.get(6)?,
        unread: r.get::<_, i64>(7)? != 0,
        ts: r.get(8)?,
    })
}

/// Insert: `INSERT OR IGNORE` on the unique `dedup_key`; `None` means a duplicate transition.
pub(crate) fn insert(
    tx: &Transaction<'_>,
    ts: i64,
    e: &NewEvent,
) -> rusqlite::Result<Option<Event>> {
    let text: String = e.text.chars().take(agentd_core::MAX_TEXT_CHARS).collect();
    let n = tx.execute(
        "INSERT OR IGNORE INTO events(kind, sub, level, text, session_id, source, dedup_key, unread, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
        params![
            e.kind,
            e.sub,
            e.level.as_str(),
            text,
            e.session_id,
            e.source,
            e.dedup_key,
            ts
        ],
    )?;
    if n == 0 {
        return Ok(None);
    }
    let id = tx.last_insert_rowid();
    tx.query_row(
        &format!("SELECT {COLS} FROM events WHERE id = ?1"),
        [id],
        event_of,
    )
    .map(Some)
}

/// Conditions by `kind`, `sub`, `session_id`: shared by selection, counters and marking.
fn scope(
    kind: Option<&str>,
    sub: Option<&str>,
    session: Option<i64>,
    conds: &mut Vec<String>,
    args: &mut Vec<Sql>,
) {
    if let Some(k) = kind {
        args.push(Sql::Text(k.to_string()));
        conds.push(format!("kind = ?{}", args.len()));
    }
    if let Some(s) = sub {
        args.push(Sql::Text(s.to_string()));
        conds.push(format!("sub = ?{}", args.len()));
    }
    if let Some(id) = session {
        args.push(Sql::Integer(id));
        conds.push(format!("session_id = ?{}", args.len()));
    }
}

fn where_of(conds: &[String]) -> String {
    if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    }
}

/// Unread counters for a selection: `{total, by_kind, by_sub}` (key `kind.sub`).
fn counts(c: &Connection, f: &EventFilter) -> rusqlite::Result<Value> {
    let (mut conds, mut args) = (vec!["unread = 1".to_string()], Vec::new());
    scope(
        f.kind.as_deref(),
        f.sub.as_deref(),
        f.session_id,
        &mut conds,
        &mut args,
    );
    let sql = format!(
        "SELECT kind, sub, count(*) FROM events{} GROUP BY kind, sub",
        where_of(&conds)
    );
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args), |r| {
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
        let cur = by_kind.get(&kind).and_then(Value::as_i64).unwrap_or(0);
        by_kind.insert(kind.clone(), json!(cur + n));
        by_sub.insert(format!("{kind}.{sub}"), json!(n));
    }
    Ok(json!({"total": total, "by_kind": by_kind, "by_sub": by_sub}))
}

/// `{items, counts}`; pages by `id` descending.
pub(super) fn list(c: &Connection, f: &EventFilter) -> rusqlite::Result<Value> {
    let (mut conds, mut args) = (Vec::new(), Vec::new());
    scope(
        f.kind.as_deref(),
        f.sub.as_deref(),
        f.session_id,
        &mut conds,
        &mut args,
    );
    if let Some(u) = f.unread {
        conds.push(format!("unread = {}", i64::from(u)));
    }
    if let Some(b) = f.before {
        args.push(Sql::Integer(b));
        conds.push(format!("id < ?{}", args.len()));
    }
    args.push(Sql::Integer(f.limit));
    let sql = format!(
        "SELECT {COLS} FROM events{} ORDER BY id DESC LIMIT ?{}",
        where_of(&conds),
        args.len()
    );
    let items = c
        .prepare(&sql)?
        .query_map(params_from_iter(args), event_of)?
        .map(|e| e.map(|e| e.to_json()))
        .collect::<rusqlite::Result<Vec<Value>>>()?;
    Ok(json!({"items": items, "counts": counts(c, f)?}))
}

/// Ids of the unread events of a selection.
fn unread_ids(tx: &Transaction<'_>, s: &ReadSelector) -> rusqlite::Result<Vec<i64>> {
    let (mut conds, mut args) = (vec!["unread = 1".to_string()], Vec::new());
    scope(
        s.kind.as_deref(),
        s.sub.as_deref(),
        s.session_id,
        &mut conds,
        &mut args,
    );
    let sql = format!("SELECT id FROM events{} ORDER BY id", where_of(&conds));
    let all = tx
        .prepare(&sql)?
        .query_map(params_from_iter(args), |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(match &s.ids {
        Some(want) => all.into_iter().filter(|id| want.contains(id)).collect(),
        None => all,
    })
}

fn set_read(tx: &Transaction<'_>, ids: &[i64]) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare("UPDATE events SET unread = 0 WHERE id = ?1")?;
    for id in ids {
        stmt.execute([id])?;
    }
    Ok(())
}

/// Marks a selection's events as read; returns their `id` (for the `read` delta).
pub(super) fn mark_read(tx: &Transaction<'_>, s: &ReadSelector) -> rusqlite::Result<Vec<i64>> {
    let ids = unread_ids(tx, s)?;
    set_read(tx, &ids)?;
    Ok(ids)
}

/// Session events are read along with the session (`POST /sessions/{id}/read`).
pub(crate) fn mark_session_events(
    tx: &Transaction<'_>,
    session: i64,
) -> rusqlite::Result<Vec<i64>> {
    mark_read(
        tx,
        &ReadSelector {
            session_id: Some(session),
            ..ReadSelector::default()
        },
    )
}

/// `Some(true)` means it was unread and is now marked; `Some(false)` means already read; `None` means no event.
pub(super) fn mark_one(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Option<bool>> {
    let unread: Option<i64> = tx
        .query_row("SELECT unread FROM events WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(unread) = unread else {
        return Ok(None);
    };
    if unread != 0 {
        set_read(tx, &[id])?;
    }
    Ok(Some(unread != 0))
}
