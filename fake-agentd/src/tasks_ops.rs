//! Tasks and runs (db.md section 3.8): parsing and validating bodies, `tasks` and `task_runs` rows, JSON `Task` and `TaskRun`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Map, Value, json};

use crate::events_db::NewEvent;
use crate::events_ops;
use crate::state::Shared;

const COLS: &str =
    "id, kind, title, spec, cron, tz, state, persona_id, session_id, next_run, created_at";

pub const RUNS_DEFAULT_LIMIT: i64 = 50;
pub const RUNS_MAX_LIMIT: i64 = 200;

fn from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
        "spec": r.get::<_, String>(3)?, "cron": r.get::<_, Option<String>>(4)?, "tz": r.get::<_, String>(5)?,
        "state": r.get::<_, String>(6)?, "persona_id": r.get::<_, Option<i64>>(7)?,
        "session_id": r.get::<_, Option<i64>>(8)?, "next_run": r.get::<_, Option<i64>>(9)?,
        "created_at": r.get::<_, i64>(10)?,
    }))
}

fn run_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "task_id": r.get::<_, i64>(1)?, "started_at": r.get::<_, i64>(2)?,
        "finished_at": r.get::<_, Option<i64>>(3)?, "status": r.get::<_, String>(4)?,
        "output": r.get::<_, Option<String>>(5)?,
    }))
}

/// Validation error: field and explanation (`422 validation_failed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bad {
    pub field: &'static str,
    pub why: &'static str,
}

fn bad(field: &'static str, why: &'static str) -> Bad {
    Bad { field, why }
}

/// Change or creation: `None` - the field was not passed.
#[derive(Debug, Default, Clone)]
pub struct Fields {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub spec: Option<String>,
    pub cron: Option<Option<String>>,
    pub state: Option<String>,
    pub persona_id: Option<Option<i64>>,
}

fn text(v: &Value, field: &'static str, max: usize) -> Result<String, Bad> {
    match v.as_str() {
        Some(s) if !s.trim().is_empty() && s.chars().count() <= max => Ok(s.to_string()),
        _ => Err(bad(field, "non-empty string of allowed length")),
    }
}

fn cron_ok(c: &str) -> bool {
    c.split_whitespace().count() == 5
}

pub fn parse(body: &Map<String, Value>, create: bool) -> Result<Fields, Bad> {
    const KNOWN: [&str; 6] = ["kind", "title", "spec", "cron", "state", "persona_id"];
    if body.keys().any(|k| !KNOWN.contains(&k.as_str())) {
        return Err(bad("body", "unknown field"));
    }
    let mut f = Fields::default();
    if let Some(v) = body.get("kind") {
        match v.as_str() {
            Some(k @ ("sched" | "deleg")) => f.kind = Some(k.to_string()),
            _ => return Err(bad("kind", "sched or deleg")),
        }
    }
    if let Some(v) = body.get("title") {
        f.title = Some(text(v, "title", 200)?);
    }
    if let Some(v) = body.get("spec") {
        f.spec = Some(text(v, "spec", 20_000)?);
    }
    if let Some(v) = body.get("cron") {
        f.cron = Some(match v {
            Value::Null => None,
            Value::String(c) if cron_ok(c) => Some(c.clone()),
            _ => return Err(bad("cron", "cron with five fields")),
        });
    }
    if let Some(v) = body.get("state") {
        match v.as_str() {
            Some(s @ ("active" | "paused" | "done")) => f.state = Some(s.to_string()),
            _ => return Err(bad("state", "active, paused or done")),
        }
    }
    if let Some(v) = body.get("persona_id") {
        f.persona_id = Some(match v {
            Value::Null => None,
            other => Some(
                other
                    .as_i64()
                    .ok_or_else(|| bad("persona_id", "integer or null"))?,
            ),
        });
    }
    if create {
        let kind = f
            .kind
            .as_deref()
            .ok_or_else(|| bad("kind", "required field"))?;
        if f.title.is_none() {
            return Err(bad("title", "required field"));
        }
        if f.spec.is_none() {
            return Err(bad("spec", "required field"));
        }
        match (kind, f.cron.as_ref().and_then(Option::as_ref)) {
            ("sched", None) => return Err(bad("cron", "sched needs a schedule")),
            ("deleg", Some(_)) => return Err(bad("cron", "deleg has no schedule")),
            _ => {}
        }
    }
    Ok(f)
}

pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM tasks WHERE id = ?1"),
        params![id],
        from_row,
    )
    .optional()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Filter<'a> {
    pub kind: Option<&'a str>,
    pub state: Option<&'a str>,
    pub session_id: Option<i64>,
}

pub fn list(conn: &Connection, f: &Filter<'_>) -> rusqlite::Result<Vec<Value>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM tasks WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR state = ?2) \
         AND (?3 IS NULL OR session_id = ?3) ORDER BY id"
    ))?;
    let rows = st.query_map(params![f.kind, f.state, f.session_id], from_row)?;
    rows.collect()
}

pub fn insert(conn: &Connection, session: i64, f: &Fields, now: i64) -> rusqlite::Result<Value> {
    conn.execute(
        "INSERT INTO tasks(kind, title, spec, cron, state, persona_id, session_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, COALESCE(?5, 'active'), ?6, ?7, ?8)",
        params![
            f.kind,
            f.title,
            f.spec,
            f.cron.clone().flatten(),
            f.state,
            f.persona_id.flatten(),
            session,
            now
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(get(conn, id)?.unwrap_or(Value::Null))
}

/// `Ok(None)` - no such task. Only the passed fields change; `kind` does not change.
pub fn update(conn: &Connection, id: i64, f: &Fields) -> rusqlite::Result<Option<Value>> {
    let Some(cur) = get(conn, id)? else {
        return Ok(None);
    };
    let pick = |new: &Option<String>, key: &str| -> String {
        new.clone()
            .unwrap_or_else(|| cur[key].as_str().unwrap_or_default().to_string())
    };
    let cron: Option<String> = match &f.cron {
        Some(c) => c.clone(),
        None => cur["cron"].as_str().map(str::to_string),
    };
    let persona = match f.persona_id {
        Some(p) => p,
        None => cur["persona_id"].as_i64(),
    };
    conn.execute(
        "UPDATE tasks SET title = ?1, spec = ?2, cron = ?3, state = ?4, persona_id = ?5 WHERE id = ?6",
        params![
            pick(&f.title, "title"),
            pick(&f.spec, "spec"),
            cron,
            pick(&f.state, "state"),
            persona,
            id
        ],
    )?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])? == 1)
}

pub fn runs(
    conn: &Connection,
    id: i64,
    before: Option<i64>,
    limit: i64,
) -> rusqlite::Result<Vec<Value>> {
    let mut st = conn.prepare(
        "SELECT id, task_id, started_at, finished_at, status, output FROM task_runs \
         WHERE task_id = ?1 AND (?2 IS NULL OR id < ?2) ORDER BY id DESC LIMIT ?3",
    )?;
    let rows = st.query_map(params![id, before, limit], run_row)?;
    rows.collect()
}

/// Run now: a `task_runs` row, `task/<kind>` (start) and `task/done` (result) events in the feed, `deleg` after the run is `done`.
pub fn run_now(shared: &Shared, id: i64) -> Result<Option<Value>, crate::DbError> {
    let now = shared.clock.now_unix();
    let made = shared.db.write(|w| {
        let Some(task) = get(w, id)? else {
            return Ok(None);
        };
        let title = task["title"].as_str().unwrap_or_default();
        w.execute(
            "INSERT INTO task_runs(task_id, started_at, finished_at, status, output) VALUES (?1, ?2, ?2, 'ok', ?3)",
            params![id, now, format!("Done: {title}")],
        )?;
        let run_id = w.last_insert_rowid();
        if task["kind"] == "deleg" {
            w.execute("UPDATE tasks SET state = 'done' WHERE id = ?1", params![id])?;
        }
        let run = w.query_row(
            "SELECT id, task_id, started_at, finished_at, status, output FROM task_runs WHERE id = ?1",
            params![run_id],
            run_row,
        )?;
        Ok(Some((task, run)))
    })?;
    let Some((task, run)) = made else {
        return Ok(None);
    };
    let title = task["title"].as_str().unwrap_or_default();
    let kind = task["kind"].as_str().unwrap_or("sched");
    let session = task["session_id"].as_i64();
    for (sub, level, text) in [
        (kind, "info", format!("Run: {title}")),
        ("done", "ok", format!("Done: {title}")),
    ] {
        events_ops::add(
            shared,
            &NewEvent {
                kind: "task",
                sub,
                level: Some(level),
                text: &text,
                session_id: session,
                source: Some("agent-sched"),
                dedup_key: None,
                ts: now,
            },
        )?;
    }
    Ok(Some(run))
}
