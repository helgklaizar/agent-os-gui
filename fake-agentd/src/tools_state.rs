//! Bridge of browser commands: open `request_id`s, the first accepted result, closing on `stop`.
//! Emulates `agentd_core::BrowserBridge`: idempotency by `request_id`, `result_exists` and `request_closed` (api.md section 5.2.1).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

struct Entry {
    session: i64,
    kind: &'static str,
    result: Option<Value>,
    closed: bool,
}

/// Result of accepting a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    /// Accepted (first time or an identical repeat).
    Accepted,
    Unknown,
    /// The result kind does not match the command kind.
    KindMismatch,
    /// Command closed (`stop`, timeout).
    Closed,
    /// A different result for the same `request_id`.
    Conflict,
}

#[derive(Default)]
pub struct ToolBridge {
    next: AtomicU64,
    open: Mutex<HashMap<String, Entry>>,
}

impl ToolBridge {
    /// New command to the browser; returns `request_id`.
    pub fn open(&self, session: i64, kind: &'static str) -> String {
        let id = format!("req-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id.clone(),
                Entry {
                    session,
                    kind,
                    result: None,
                    closed: false,
                },
            );
        id
    }

    pub fn submit(&self, session: i64, request_id: &str, kind: &str, body: &Value) -> Submit {
        let mut map = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(e) = map.get_mut(request_id).filter(|e| e.session == session) else {
            return Submit::Unknown;
        };
        if e.kind != kind {
            return Submit::KindMismatch;
        }
        match &e.result {
            Some(prev) if prev == body => Submit::Accepted,
            Some(_) => Submit::Conflict,
            None if e.closed => Submit::Closed,
            None => {
                e.result = Some(body.clone());
                Submit::Accepted
            }
        }
    }

    /// The session's open commands are closed (`stop`): a result for them is no longer accepted.
    pub fn close_session(&self, session: i64) {
        for e in self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values_mut()
            .filter(|e| e.session == session && e.result.is_none())
        {
            e.closed = true;
        }
    }
}
