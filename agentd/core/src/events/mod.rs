//! Event feed (`docs/reference/db.md` 3.3, `docs/reference/api.md` 5.3): types, store port and the subtype registry. No I/O here.

mod parse;
mod subtypes;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{FieldError, PortFuture};

pub use parse::{parse_filter, parse_read};
pub use subtypes::{SUBTYPES, Subtype, subtypes_json};

/// Values of `events.kind` (schema `CHECK`).
pub const KINDS: [&str; 3] = ["ai", "task", "msg"];
/// Event text length limit: longer text is truncated, the event is not lost.
pub const MAX_TEXT_CHARS: usize = 1000;
pub const MAX_SUB_CHARS: usize = 32;
/// Default page of `GET /events` and its limit.
pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 200;
/// How many identifiers `POST /events/read` accepts.
pub const MAX_READ_IDS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventLevel {
    Info,
    Warn,
    Error,
    Ok,
}

impl EventLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EventLevel::Info => "info",
            EventLevel::Warn => "warn",
            EventLevel::Error => "error",
            EventLevel::Ok => "ok",
        }
    }
}

/// New event. `dedup_key` gives "one transition — one event" (`INSERT OR IGNORE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub kind: &'static str,
    pub sub: String,
    pub level: EventLevel,
    pub text: String,
    pub session_id: Option<i64>,
    pub source: Option<String>,
    pub dedup_key: Option<String>,
}

impl NewEvent {
    #[must_use]
    pub fn new(
        kind: &'static str,
        sub: &str,
        level: EventLevel,
        text: impl Into<String>,
    ) -> NewEvent {
        NewEvent {
            kind,
            sub: sub.to_string(),
            level,
            text: text.into(),
            session_id: None,
            source: Some("agentd".to_string()),
            dedup_key: None,
        }
    }

    #[must_use]
    pub fn session(mut self, id: Option<i64>) -> NewEvent {
        self.session_id = id;
        self
    }

    #[must_use]
    pub fn dedup(mut self, key: String) -> NewEvent {
        self.dedup_key = Some(key);
        self
    }
}

/// Feed row; the DB has no `tab` field, the core computes it on output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    pub kind: String,
    pub sub: String,
    pub level: String,
    pub text: String,
    pub session_id: Option<i64>,
    pub source: Option<String>,
    pub unread: bool,
    pub ts: i64,
}

/// The tab that "Open" leads to: chat by default.
#[must_use]
pub fn tab_of(_kind: &str, _sub: &str) -> &'static str {
    "chat"
}

impl Event {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "kind": self.kind, "sub": self.sub, "level": self.level,
            "text": self.text, "session_id": self.session_id,
            "tab": tab_of(&self.kind, &self.sub), "source": self.source,
            "unread": self.unread, "ts": self.ts,
        })
    }
}

/// `GET /events` filter; `before` is by `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFilter {
    pub kind: Option<String>,
    pub sub: Option<String>,
    pub unread: Option<bool>,
    pub session_id: Option<i64>,
    pub before: Option<i64>,
    pub limit: i64,
}

impl Default for EventFilter {
    fn default() -> EventFilter {
        EventFilter {
            kind: None,
            sub: None,
            unread: None,
            session_id: None,
            before: None,
            limit: DEFAULT_LIMIT,
        }
    }
}

/// Event selection for `POST /events/read`; all fields empty — all unread.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadSelector {
    pub ids: Option<Vec<i64>>,
    pub kind: Option<String>,
    pub sub: Option<String>,
    pub session_id: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub enum EventError {
    NotFound,
    Invalid(Vec<FieldError>),
    Unavailable,
}

pub type EventResult<T> = Result<T, EventError>;

/// Feed store. Write and publish (`event`, `delta {read}`) are one operation: the delta goes out after commit.
pub trait EventStore: Send + Sync + 'static {
    /// `Ok(None)` — an event with this `dedup_key` already exists, no new one was created.
    fn emit(&self, event: NewEvent) -> PortFuture<'_, EventResult<Option<Value>>>;
    /// `{items, counts}`; `counts` — unread over the whole selection without `before`, `limit` and `unread`.
    fn list(&self, filter: EventFilter) -> PortFuture<'_, EventResult<Value>>;
    /// How many events became read.
    fn mark_read(&self, selector: ReadSelector) -> PortFuture<'_, EventResult<usize>>;
    /// Repeating on a read event is a success; no such event — `NotFound`.
    fn mark_one(&self, id: i64) -> PortFuture<'_, EventResult<()>>;
    /// `snapshot` of the `events` topic: latest events and counters.
    fn snapshot(&self) -> PortFuture<'_, EventResult<Value>>;
}

/// Emitter for event sources: a write error must not bring the source down, it goes to the log.
#[derive(Clone)]
pub struct Events {
    store: Arc<dyn EventStore>,
}

impl Events {
    #[must_use]
    pub fn new(store: Arc<dyn EventStore>) -> Events {
        Events { store }
    }

    /// `emit(kind, sub, level, text, session_id, source, dedup_key?)`.
    pub async fn emit(&self, event: NewEvent) -> Option<Value> {
        match self.store.emit(event).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(scope = "events", event = "emit_failed", error = ?e, "event not written");
                None
            }
        }
    }

    #[must_use]
    pub fn store(&self) -> &Arc<dyn EventStore> {
        &self.store
    }
}
