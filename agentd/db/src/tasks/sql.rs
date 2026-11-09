//! Task and run SQL (A5-09): tasks and runs in `sessions.db`; writes go through the single writer.

use agentd_core::{FieldError, NewTask, Task, TaskError, TaskFilter, TaskRun};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};

pub(super) const TASK_COLS: &str =
    "id, kind, title, spec, cron, tz, state, persona_id, session_id, next_run, created_at";
pub(super) const RUN_COLS: &str = "id, task_id, started_at, finished_at, status, output";

pub(super) fn task_of(r: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        spec: r.get(3)?,
        cron: r.get(4)?,
        tz: r.get(5)?,
        state: r.get(6)?,
        persona_id: r.get(7)?,
        session_id: r.get(8)?,
        next_run: r.get(9)?,
        created_at: r.get(10)?,
    })
}

pub(super) fn run_of(r: &Row<'_>) -> rusqlite::Result<TaskRun> {
    Ok(TaskRun {
        id: r.get(0)?,
        task_id: r.get(1)?,
        started_at: r.get(2)?,
        finished_at: r.get(3)?,
        status: r.get(4)?,
        output: r.get(5)?,
    })
}

pub(super) fn get(c: &Connection, id: i64) -> rusqlite::Result<Option<Task>> {
    c.query_row(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"),
        [id],
        task_of,
    )
    .optional()
}

/// Write result: `Err(TaskError)` is a domain refusal, inside `Ok` is the transaction's success.
pub(super) type Done<T> = rusqlite::Result<Result<T, TaskError>>;

pub(super) fn insert(tx: &Transaction<'_>, now: i64, t: &NewTask) -> Done<Task> {
    let live: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sessions WHERE id = ?1",
            [t.session_id],
            |r| r.get(0),
        )
        .optional()?;
    if live.is_none() {
        return Ok(Err(TaskError::SessionNotFound));
    }
    if let Some(p) = t.persona_id {
        let ok: Option<i64> = tx
            .query_row("SELECT 1 FROM personas WHERE id = ?1", [p], |r| r.get(0))
            .optional()?;
        if ok.is_none() {
            return Ok(Err(TaskError::Invalid(vec![FieldError::new(
                "persona_id",
                "no such persona",
            )])));
        }
    }
    tx.execute(
        "INSERT INTO tasks(kind, title, spec, cron, tz, state, persona_id, session_id, next_run, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9)",
        params![t.kind, t.title, t.spec, t.cron, t.tz, t.persona_id, t.session_id, t.next_run, now],
    )?;
    Ok(Ok(
        get(tx, tx.last_insert_rowid())?.ok_or(rusqlite::Error::QueryReturnedNoRows)?
    ))
}

pub(super) fn list(c: &Connection, f: &TaskFilter) -> rusqlite::Result<Vec<Task>> {
    let (mut conds, mut args): (Vec<String>, Vec<Sql>) = (Vec::new(), Vec::new());
    if let Some(k) = &f.kind {
        args.push(Sql::Text(k.clone()));
        conds.push(format!("kind = ?{}", args.len()));
    }
    if let Some(s) = &f.state {
        args.push(Sql::Text(s.clone()));
        conds.push(format!("state = ?{}", args.len()));
    }
    if let Some(s) = f.session_id {
        args.push(Sql::Integer(s));
        conds.push(format!("session_id = ?{}", args.len()));
    }
    let wh = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    c.prepare(&format!("SELECT {TASK_COLS} FROM tasks{wh} ORDER BY id"))?
        .query_map(params_from_iter(args), task_of)?
        .collect()
}
