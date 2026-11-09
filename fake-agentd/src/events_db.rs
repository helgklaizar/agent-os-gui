//! Writing events to `events`: insert, indicator transition. Only the single DB writer writes.

use rusqlite::{Connection, OptionalExtension, params};

/// New event; `session_id = None` - system.
#[derive(Debug, Clone, Default)]
pub struct NewEvent<'a> {
    pub kind: &'a str,
    pub sub: &'a str,
    pub level: Option<&'a str>,
    pub text: &'a str,
    pub session_id: Option<i64>,
    pub source: Option<&'a str>,
    pub dedup_key: Option<&'a str>,
    pub ts: i64,
}

/// Insert with `INSERT OR IGNORE`: a repeated `dedup_key` creates no event (`None`).
pub fn insert(conn: &Connection, e: &NewEvent<'_>) -> rusqlite::Result<Option<i64>> {
    let rows = conn.execute(
        "INSERT OR IGNORE INTO events(kind, sub, level, text, session_id, source, dedup_key, ts) \
         VALUES (?1, ?2, COALESCE(?3, 'info'), ?4, ?5, ?6, ?7, ?8)",
        params![
            e.kind,
            e.sub,
            e.level,
            e.text,
            e.session_id,
            e.source,
            e.dedup_key,
            e.ts
        ],
    )?;
    Ok((rows == 1).then(|| conn.last_insert_rowid()))
}

fn level_of(state: &str) -> &'static str {
    match state {
        "bad" => "error",
        "warn" => "warn",
        "ok" => "ok",
        _ => "info",
    }
}

/// Indicator state transition (db.md section 3.3): in one transaction `transition_seq + 1`, the new state and an event
/// `dedup_key = <indicator>:<transition_seq>`. A repeated frame in the same state creates no event (`None`).
/// An unknown indicator or a state outside the set is an error.
pub fn transition(
    conn: &Connection,
    indicator: &str,
    state: &str,
    now: i64,
) -> rusqlite::Result<Option<i64>> {
    let tx = conn.unchecked_transaction()?;
    let row: Option<(String, i64)> = tx
        .query_row(
            "SELECT last_state, transition_seq FROM indicators WHERE id = ?1",
            [indicator],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((last, seq)) = row else {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    };
    if last == state {
        return Ok(None);
    }
    let seq = seq + 1;
    tx.execute(
        "UPDATE indicators SET last_state = ?1, transition_seq = ?2, last_change_at = ?3 WHERE id = ?4",
        params![state, seq, now, indicator],
    )?;
    let key = format!("{indicator}:{seq}");
    let text = format!("{indicator}: {last} → {state}");
    let id = insert(
        &tx,
        &NewEvent {
            kind: "msg",
            sub: "sys",
            level: Some(level_of(state)),
            text: &text,
            source: Some("agentd"),
            dedup_key: Some(&key),
            ts: now,
            ..NewEvent::default()
        },
    )?;
    tx.commit()?;
    Ok(id)
}
