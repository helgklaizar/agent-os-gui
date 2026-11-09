//! Mock chat state: permission requests, attachments, stream control.
//!
//! A permission request lives in memory and in `actions_log` (request id = `actions_log.id`); the card is a `role = system` message.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// Request lifetime: 10 minutes on the virtual clock.
pub const REQUEST_TTL_SECS: i64 = 600;

#[derive(Debug, Clone)]
pub struct Req {
    pub id: i64,
    pub session_id: Option<i64>,
    pub persona_id: Option<i64>,
    pub action: String,
    pub target: Option<String>,
    /// `policy` or `observer`.
    pub source: &'static str,
    /// `ask` or `warn`.
    pub level: &'static str,
    pub kind: Option<String>,
    pub verdict: Option<String>,
    /// `pending`, `allowed`, `denied`, `expired`, `cancelled`.
    pub status: String,
    pub detail: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<i64>,
    pub expires_at: i64,
    pub created_at: i64,
    pub message_id: Option<i64>,
    pub finding_id: Option<i64>,
}

impl Req {
    /// `PermissionRequest` per the api.md section 6 schema.
    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "session_id": self.session_id, "action": self.action, "target": self.target,
            "source": self.source, "level": self.level, "kind": self.kind, "verdict": self.verdict,
            "status": self.status, "detail": self.detail, "decided_by": self.decided_by,
            "decided_at": self.decided_at, "expires_at": self.expires_at, "created_at": self.created_at,
        })
    }
}

pub struct Blob {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct ChatInner {
    pub requests: BTreeMap<i64, Req>,
    pub attachments: HashMap<i64, Blob>,
    pub next_attachment: i64,
    /// Stop flag of the running reply for a session.
    pub streams: HashMap<i64, Arc<AtomicBool>>,
}

pub struct ChatState {
    reply_delay: Duration,
    observer_up: AtomicBool,
    inner: Mutex<ChatInner>,
}

impl ChatState {
    pub fn new(reply_delay: Duration) -> ChatState {
        ChatState {
            reply_delay,
            observer_up: AtomicBool::new(true),
            inner: Mutex::new(ChatInner::default()),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, ChatInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn reply_delay(&self) -> Duration {
        self.reply_delay
    }

    pub fn observer_up(&self) -> bool {
        self.observer_up.load(Ordering::SeqCst)
    }

    pub fn set_observer_up(&self, up: bool) {
        self.observer_up.store(up, Ordering::SeqCst);
    }

    /// Registers a new reply for the session; the previous flag stays with the old reply.
    pub fn begin_stream(&self, session: i64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.lock().streams.insert(session, flag.clone());
        flag
    }

    pub fn end_stream(&self, session: i64, flag: &Arc<AtomicBool>) {
        let mut g = self.lock();
        if g.streams
            .get(&session)
            .is_some_and(|f| Arc::ptr_eq(f, flag))
        {
            g.streams.remove(&session);
        }
    }

    /// Sets the stop flag; `true` if a reply was running.
    pub fn stop_stream(&self, session: i64) -> bool {
        match self.lock().streams.get(&session) {
            Some(f) => {
                f.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    pub fn pending(&self) -> Vec<Req> {
        self.lock()
            .requests
            .values()
            .filter(|r| r.status == "pending")
            .cloned()
            .collect()
    }

    pub fn open_for(&self, session: i64) -> Vec<Value> {
        self.lock()
            .requests
            .values()
            .filter(|r| r.status == "pending" && r.session_id == Some(session))
            .map(Req::json)
            .collect()
    }
}
