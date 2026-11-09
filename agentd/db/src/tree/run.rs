//! Executing a write operation: transaction, mapping DB errors to `TreeError`, publishing events after commit.

use agentd_core::{TreeError, TreeResult};
use rusqlite::Transaction;

use super::{Event, TreeRepo};
use crate::DbError;

/// Outcome of an operation inside a transaction.
pub(crate) struct Done<T> {
    pub value: T,
    pub events: Vec<Event>,
    /// `secret_ref` of accounts whose secrets must be erased after commit.
    pub secrets: Vec<String>,
}

impl<T> Done<T> {
    pub fn new(value: T, events: Vec<Event>) -> Done<T> {
        Done {
            value,
            events,
            secrets: Vec::new(),
        }
    }
}

/// A domain-rule refusal is returned as a value before the first write, so the transaction rolls back without traces.
pub(crate) type Op<T> = Result<Done<T>, TreeError>;

/// `RAISE(ABORT, …)` messages from the 0003 triggers (docs/reference/db.md section 3.1).
fn from_trigger(msg: &str) -> Option<TreeError> {
    if msg.contains("folder cycle") {
        Some(TreeError::Conflict("folder_cycle"))
    } else if msg.contains("folder depth") {
        Some(TreeError::field("parent_id", "folder depth is at most 8"))
    } else if msg.contains("thread depth") {
        Some(TreeError::field(
            "parent_id",
            "a thread cannot have threads",
        ))
    } else {
        None
    }
}

pub(crate) fn map_db(e: &DbError) -> TreeError {
    if let DbError::Sqlite(rusqlite::Error::SqliteFailure(_, Some(msg))) = e
        && let Some(t) = from_trigger(msg)
    {
        return t;
    }
    tracing::warn!(scope = "tree", event = "db_unavailable", error = %e, "tree DB unavailable");
    TreeError::Unavailable
}

impl TreeRepo {
    pub(crate) fn now(&self) -> i64 {
        self.clock.now_unix()
    }

    pub(crate) fn home(&self) -> Option<&std::path::Path> {
        self.home.as_deref()
    }

    pub(crate) async fn run<T, F>(&self, f: F) -> TreeResult<T>
    where
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<Op<T>> + Send + 'static,
        T: Send + 'static,
    {
        let done = self.db.write(f).await.map_err(|e| map_db(&e))??;
        for (topic, data) in done.events {
            // api.md section 4.4: a session settings change also goes to its `session.<id>` topic (`kind = session.updated`).
            if topic == "sessions"
                && data["op"] == "session.updated"
                && let Some(id) = data["session"]["id"].as_i64()
            {
                self.notifier.publish_event(
                    &format!("session.{id}"),
                    serde_json::json!({"kind": "session.updated", "session": data["session"]}),
                );
            }
            self.notifier.publish(topic, data);
        }
        if let Some(eraser) = &self.eraser {
            for secret in &done.secrets {
                eraser.erase(secret).await;
            }
        }
        Ok(done.value)
    }

    pub(crate) async fn read<T, F>(&self, f: F) -> TreeResult<T>
    where
        F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.db.read(f).await.map_err(|e| map_db(&e))
    }
}
