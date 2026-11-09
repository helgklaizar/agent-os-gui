//! Browser commands and `tool-results`: the core issues a `request_id`, sends `browser.command` to the session subscribers and
//! accepts the result only from the token that received the command; repeating is idempotent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::watch;

use crate::{Clock, Notifier};

mod exec;
mod result;

pub use exec::BrowseExecutor;
pub use result::{MAX_OUTPUT_BYTES, ToolResult, parse_tool_result};

/// Browser command timeout, seconds.
pub const BROWSE_TIMEOUT_SECS: i64 = 60;

/// Result rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// No command with this `request_id` in this session (`404`).
    Unknown,
    /// The token did not receive the event with this `request_id` (`403`).
    Foreign,
    /// The command is closed: expired, cancelled, session deleted (`409 request_closed`).
    Closed,
    /// A different result for the same `request_id` (`409 result_exists`).
    Conflict,
    /// The result `kind` does not match the command kind (`422`).
    KindMismatch,
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserOutcome {
    Result(Box<ToolResult>),
    /// `timeout`, `cancelled`, `session_deleted`.
    Closed(&'static str),
}

enum State {
    Open,
    Done(Box<ToolResult>),
    Closed,
}

struct Command {
    session_id: i64,
    kind: &'static str,
    owners: Vec<i64>,
    state: State,
    deadline: i64,
    tx: watch::Sender<Option<BrowserOutcome>>,
}

struct Inner {
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
    commands: Mutex<HashMap<String, Command>>,
}

/// Browser command registry.
#[derive(Clone)]
pub struct BrowserBridge {
    inner: Arc<Inner>,
}

impl BrowserBridge {
    #[must_use]
    pub fn new(notifier: Arc<dyn Notifier>, clock: Arc<dyn Clock>) -> BrowserBridge {
        BrowserBridge {
            inner: Arc::new(Inner {
                notifier,
                clock,
                commands: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn commands(&self) -> std::sync::MutexGuard<'_, HashMap<String, Command>> {
        self.inner
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Issues a `request_id`, remembers the recipients (tokens subscribed to `session.<id>`) and sends `browser.command`.
    /// No subscribers — `Err("no_browser_client")`: nobody to execute the command.
    pub fn open(
        &self,
        session_id: i64,
        command: &str,
        url: &str,
    ) -> Result<(String, watch::Receiver<Option<BrowserOutcome>>), &'static str> {
        let topic = format!("session.{session_id}");
        let owners = self.inner.notifier.watcher_tokens(&topic);
        if owners.is_empty() {
            return Err("no_browser_client");
        }
        let request_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = watch::channel(None);
        self.commands().insert(
            request_id.clone(),
            Command {
                session_id,
                kind: "browser",
                owners,
                state: State::Open,
                deadline: self.inner.clock.now_unix() + BROWSE_TIMEOUT_SECS,
                tx,
            },
        );
        self.inner.notifier.publish_event(
            &topic,
            json!({"kind": "browser.command", "command": command, "url": url, "request_id": request_id}),
        );
        Ok((request_id, rx))
    }

    /// Accepts a result. Order of checks: unknown command, foreign token, closed, repeat or conflict.
    pub fn submit(
        &self,
        session_id: i64,
        token_id: i64,
        res: ToolResult,
    ) -> Result<(), SubmitError> {
        let mut map = self.commands();
        let Some(cmd) = map
            .get_mut(&res.request_id)
            .filter(|c| c.session_id == session_id)
        else {
            return Err(SubmitError::Unknown);
        };
        if !cmd.owners.contains(&token_id) {
            return Err(SubmitError::Foreign);
        }
        if res.kind != cmd.kind {
            return Err(SubmitError::KindMismatch);
        }
        match &cmd.state {
            State::Closed => Err(SubmitError::Closed),
            State::Done(prev) if **prev == res => Ok(()),
            State::Done(_) => Err(SubmitError::Conflict),
            State::Open => {
                cmd.state = State::Done(Box::new(res.clone()));
                // The waiter may be gone (turn stopped): the result is accepted and saved.
                drop(cmd.tx.send(Some(BrowserOutcome::Result(Box::new(res)))));
                Ok(())
            }
        }
    }

    fn close(&self, request_id: &str, reason: &'static str) {
        if let Some(cmd) = self.commands().get_mut(request_id)
            && matches!(cmd.state, State::Open)
        {
            cmd.state = State::Closed;
            drop(cmd.tx.send(Some(BrowserOutcome::Closed(reason))));
        }
    }

    /// Closes commands whose time has run out: the waiter gets `timeout`, a late result gets `request_closed`.
    pub fn expire_due(&self) {
        let now = self.inner.clock.now_unix();
        let due: Vec<String> = self
            .commands()
            .iter()
            .filter(|(_, c)| matches!(c.state, State::Open) && c.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.close(&id, "timeout");
        }
    }

    /// Session deletion: its open commands are closed.
    pub fn close_session(&self, session_id: i64) {
        let ids: Vec<String> = self
            .commands()
            .iter()
            .filter(|(_, c)| c.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close(&id, "session_deleted");
        }
    }

    /// Background timeout task: every 250 ms by the core clock.
    pub fn spawn_sweeper(&self) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                me.expire_due();
            }
        })
    }
}
