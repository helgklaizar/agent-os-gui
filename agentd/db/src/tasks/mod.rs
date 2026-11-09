//! `TaskStore` (A5-09): tasks and runs in `sessions.db`; writes go through the single writer.

use std::sync::Arc;

use agentd_core::{
    Clock, FieldError, NewTask, PortFuture, Task, TaskError, TaskFilter, TaskResult, TaskRun,
    TaskStore, TaskUpdate,
};
use rusqlite::{Connection, params};

use crate::{Db, DbError};

mod sql;

use sql::{RUN_COLS, TASK_COLS, get, insert, list, run_of, task_of};

pub struct TaskRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
}

impl TaskRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> TaskRepo {
        TaskRepo { db, clock }
    }
}

/// Session tasks for the `snapshot` of the `session.<id>` topic.
pub(crate) fn session_tasks(
    c: &Connection,
    session_id: i64,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let f = TaskFilter {
        session_id: Some(session_id),
        ..TaskFilter::default()
    };
    Ok(list(c, &f)?.iter().map(Task::to_json).collect())
}

fn unavailable(e: &DbError) -> TaskError {
    tracing::warn!(scope = "tasks", event = "db_unavailable", error = %e, "tasks DB unavailable");
    TaskError::Unavailable
}

impl TaskStore for TaskRepo {
    fn create(&self, t: NewTask) -> PortFuture<'_, TaskResult<Task>> {
        Box::pin(async move {
            let now = self.clock.now_unix();
            self.db
                .write(move |tx| insert(tx, now, &t))
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn get(&self, id: i64) -> PortFuture<'_, TaskResult<Task>> {
        Box::pin(async move {
            self.db
                .read(move |c| get(c, id))
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(TaskError::NotFound)
        })
    }

    fn list(&self, f: TaskFilter) -> PortFuture<'_, TaskResult<Vec<Task>>> {
        Box::pin(async move {
            self.db
                .read(move |c| list(c, &f))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn update(&self, id: i64, u: TaskUpdate) -> PortFuture<'_, TaskResult<Task>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE tasks SET title = ?1, spec = ?2, cron = ?3, tz = ?4, state = ?5, persona_id = ?6,
                         next_run = ?7 WHERE id = ?8",
                        params![u.title, u.spec, u.cron, u.tz, u.state, u.persona_id, u.next_run, id],
                    );
                    match n {
                        Ok(0) => Ok(Err(TaskError::NotFound)),
                        Ok(_) => Ok(get(tx, id)?.ok_or(TaskError::NotFound)),
                        Err(rusqlite::Error::SqliteFailure(e, _))
                            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                        {
                            Ok(Err(TaskError::Invalid(vec![FieldError::new(
                                "persona_id",
                                "no such persona",
                            )])))
                        }
                        Err(e) => Err(e),
                    }
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn delete(&self, id: i64) -> PortFuture<'_, TaskResult<Task>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let Some(t) = get(tx, id)? else {
                        return Ok(Err(TaskError::NotFound));
                    };
                    tx.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
                    Ok(Ok(t))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn runs(
        &self,
        task_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> PortFuture<'_, TaskResult<Vec<TaskRun>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    if get(c, task_id)?.is_none() {
                        return Ok(Err(TaskError::NotFound));
                    }
                    let rows = c
                        .prepare(&format!(
                            "SELECT {RUN_COLS} FROM task_runs WHERE task_id = ?1 AND (?2 IS NULL OR id < ?2)
                             ORDER BY id DESC LIMIT ?3"
                        ))?
                        .query_map(params![task_id, before, limit], run_of)?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(Ok(rows))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn due(&self, now: i64) -> PortFuture<'_, TaskResult<Vec<Task>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    c.prepare(&format!(
                        "SELECT {TASK_COLS} FROM tasks WHERE state = 'active' AND next_run IS NOT NULL AND next_run <= ?1
                         ORDER BY next_run, id"
                    ))?
                    .query_map([now], task_of)?
                    .collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn set_next(
        &self,
        id: i64,
        next_run: Option<i64>,
        state: Option<&'static str>,
    ) -> PortFuture<'_, TaskResult<Task>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE tasks SET next_run = ?1, state = COALESCE(?2, state) WHERE id = ?3",
                        params![next_run, state, id],
                    )?;
                    if n == 0 {
                        return Ok(Err(TaskError::NotFound));
                    }
                    Ok(get(tx, id)?.ok_or(TaskError::NotFound))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn start_run(
        &self,
        task_id: i64,
        now: i64,
        status: &'static str,
    ) -> PortFuture<'_, TaskResult<TaskRun>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    if get(tx, task_id)?.is_none() {
                        return Ok(Err(TaskError::NotFound));
                    }
                    tx.execute(
                        "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1, ?2, ?3)",
                        params![task_id, now, status],
                    )?;
                    let id = tx.last_insert_rowid();
                    Ok(Ok(tx.query_row(
                        &format!("SELECT {RUN_COLS} FROM task_runs WHERE id = ?1"),
                        [id],
                        run_of,
                    )?))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn finish_run(
        &self,
        run_id: i64,
        now: i64,
        status: &'static str,
        output: Option<String>,
    ) -> PortFuture<'_, TaskResult<TaskRun>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE task_runs SET finished_at = ?1, status = ?2, output = ?3 WHERE id = ?4",
                        params![now, status, output, run_id],
                    )?;
                    if n == 0 {
                        // The task was deleted during the run: the history went away with it.
                        return Ok(Err(TaskError::NotFound));
                    }
                    Ok(Ok(tx.query_row(
                        &format!("SELECT {RUN_COLS} FROM task_runs WHERE id = ?1"),
                        [run_id],
                        run_of,
                    )?))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn fail_stale_runs(&self, now: i64) -> PortFuture<'_, TaskResult<u64>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE task_runs SET status = 'error', finished_at = ?1, output = 'core_restart'
                         WHERE status = 'running'",
                        [now],
                    )?;
                    Ok(Ok(n as u64))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }
}
