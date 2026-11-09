//! Tasks and schedule (see docs/reference/db.md and docs/reference/api.md): types, the storage port, request body validation, the scheduler.

mod cron;
mod parse;
mod runner;
mod service;

use serde_json::{Value, json};

use crate::{FieldError, PortFuture};

pub use cron::{CronError, cron_check, cron_next};
pub use parse::{TaskCreate, TaskEdit, parse_create, parse_edit, parse_filter};
pub use service::{DEFAULT_MAX_PARALLEL, MISSED_GRACE_SECS, Tasks};

/// Default zone (api.md, `Task` schema).
pub const DEFAULT_TZ: &str = "Asia/Jerusalem";
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_SPEC_CHARS: usize = 8000;
/// Default page size and limit of `GET /tasks/{id}/runs`.
pub const RUNS_DEFAULT_LIMIT: i64 = 50;
pub const RUNS_MAX_LIMIT: i64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub spec: String,
    pub cron: Option<String>,
    pub tz: String,
    pub state: String,
    pub persona_id: Option<i64>,
    pub session_id: Option<i64>,
    pub next_run: Option<i64>,
    pub created_at: i64,
}

impl Task {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "kind": self.kind, "title": self.title, "spec": self.spec, "cron": self.cron,
            "tz": self.tz, "state": self.state, "persona_id": self.persona_id, "session_id": self.session_id,
            "next_run": self.next_run, "created_at": self.created_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRun {
    pub id: i64,
    pub task_id: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub output: Option<String>,
}

impl TaskRun {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "task_id": self.task_id, "started_at": self.started_at,
            "finished_at": self.finished_at, "status": self.status, "output": self.output,
        })
    }
}

/// New task after validation; `next_run` is calculated by the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub kind: String,
    pub title: String,
    pub spec: String,
    pub cron: Option<String>,
    pub tz: String,
    pub persona_id: Option<i64>,
    pub session_id: i64,
    pub next_run: Option<i64>,
}

/// Final state of the editable fields: the service merges the edit with the current task and calculates `next_run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdate {
    pub title: String,
    pub spec: String,
    pub cron: Option<String>,
    pub tz: String,
    pub state: String,
    pub persona_id: Option<i64>,
    pub next_run: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFilter {
    pub kind: Option<String>,
    pub state: Option<String>,
    pub session_id: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub enum TaskError {
    NotFound,
    SessionNotFound,
    Invalid(Vec<FieldError>),
    /// The task has no session (deleted): nothing to run.
    NoSession,
    Unavailable,
}

pub type TaskResult<T> = Result<T, TaskError>;

/// Task and run storage (implemented by `agentd-db`).
pub trait TaskStore: Send + Sync + 'static {
    fn create(&self, t: NewTask) -> PortFuture<'_, TaskResult<Task>>;
    fn get(&self, id: i64) -> PortFuture<'_, TaskResult<Task>>;
    fn list(&self, f: TaskFilter) -> PortFuture<'_, TaskResult<Vec<Task>>>;
    fn update(&self, id: i64, u: TaskUpdate) -> PortFuture<'_, TaskResult<Task>>;
    /// Deletes the task together with its run history.
    fn delete(&self, id: i64) -> PortFuture<'_, TaskResult<Task>>;
    /// Runs from newest to oldest, `before` is by `id`.
    fn runs(
        &self,
        task_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> PortFuture<'_, TaskResult<Vec<TaskRun>>>;
    /// Active tasks with `next_run <= now`.
    fn due(&self, now: i64) -> PortFuture<'_, TaskResult<Vec<Task>>>;
    /// Advances `next_run` (and the state) after a run or skip.
    fn set_next(
        &self,
        id: i64,
        next_run: Option<i64>,
        state: Option<&'static str>,
    ) -> PortFuture<'_, TaskResult<Task>>;
    fn start_run(
        &self,
        task_id: i64,
        now: i64,
        status: &'static str,
    ) -> PortFuture<'_, TaskResult<TaskRun>>;
    fn finish_run(
        &self,
        run_id: i64,
        now: i64,
        status: &'static str,
        output: Option<String>,
    ) -> PortFuture<'_, TaskResult<TaskRun>>;
    /// Runs left `running` after a core crash: moved to `error` at start.
    fn fail_stale_runs(&self, now: i64) -> PortFuture<'_, TaskResult<u64>>;
}
