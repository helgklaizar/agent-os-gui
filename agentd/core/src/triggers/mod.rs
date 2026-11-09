//! Triggers on incoming messages (`docs/reference/db.md` 3.8, `docs/reference/api.md` 5.8): types, store port, body validation, service.
//! The action is an untrusted message into an inbox session and starting an agent turn on behalf of `cron` with untrusted context.

mod parse;
mod service;

use serde_json::{Value, json};

use crate::{FieldError, PortFuture};

pub use parse::{TriggerCreate, TriggerEdit, parse_create, parse_edit};
pub use service::{STORM_LIMIT, STORM_WINDOW_SECS, Triggers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub id: i64,
    pub source: String,
    /// `{from?, contains?}`; an empty object — any message from the source.
    pub match_: Value,
    pub session_id: Option<i64>,
    /// The "create run" action: the task the trigger starts after delivering the message to the session.
    pub task_id: Option<i64>,
    pub enabled: bool,
}

impl Trigger {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut doc = json!({"id": self.id, "source": self.source, "match": self.match_,
               "session_id": self.session_id, "enabled": self.enabled});
        if let (Some(t), Value::Object(m)) = (self.task_id, &mut doc) {
            m.insert("task_id".into(), t.into());
        }
        doc
    }
}

#[derive(Debug, PartialEq)]
pub enum TriggerError {
    NotFound,
    SessionNotFound,
    /// `task_id` points to a nonexistent task.
    TaskNotFound,
    Invalid(Vec<FieldError>),
    Unavailable,
}

pub type TriggerResult<T> = Result<T, TriggerError>;

/// Trigger store (implemented by `agentd-db`).
pub trait TriggerStore: Send + Sync + 'static {
    fn create(&self, t: Trigger) -> PortFuture<'_, TriggerResult<Trigger>>;
    fn get(&self, id: i64) -> PortFuture<'_, TriggerResult<Trigger>>;
    fn list(&self) -> PortFuture<'_, TriggerResult<Vec<Trigger>>>;
    /// Writes all mutable fields (`id` comes from the argument).
    fn update(&self, id: i64, t: Trigger) -> PortFuture<'_, TriggerResult<Trigger>>;
    fn delete(&self, id: i64) -> PortFuture<'_, TriggerResult<()>>;
}
