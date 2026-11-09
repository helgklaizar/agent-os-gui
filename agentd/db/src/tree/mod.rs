//! Store of the session, persona and account tree (`TreeStore`, A2). Writes go through the single writer: each
//! operation is one transaction; WS deltas are collected inside it and go to subscribers only after commit, so
//! a rollback leaves no false events, and a successful mutation gives exactly one delta (plus derived ones: objects
//! whose effective view changed).

mod accounts;
mod folders;
mod json;
mod personas;
mod run;
mod sessions;
mod store;

pub(crate) use json::session as session_json;

use std::path::PathBuf;
use std::sync::Arc;

use agentd_core::{Clock, Notifier, SecretEraser};

use crate::Db;

/// Event for a topic: published after commit.
pub(crate) type Event = (&'static str, serde_json::Value);

pub struct TreeRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    notifier: Arc<dyn Notifier>,
    eraser: Option<Arc<dyn SecretEraser>>,
    /// The user's home directory: the boundary for checking symlinks in `path`.
    home: Option<PathBuf>,
}

impl TreeRepo {
    #[must_use]
    pub fn new(
        db: Arc<Db>,
        clock: Arc<dyn Clock>,
        notifier: Arc<dyn Notifier>,
        home: Option<PathBuf>,
    ) -> TreeRepo {
        TreeRepo {
            db,
            clock,
            notifier,
            eraser: None,
            home,
        }
    }

    /// Wires up secret erasure on account deletion (implementation: A3-02).
    #[must_use]
    pub fn with_secret_eraser(mut self, eraser: Arc<dyn SecretEraser>) -> TreeRepo {
        self.eraser = Some(eraser);
        self
    }
}
