//! Permission requests: opening, state, waiting for a decision, one-time grants for HTTP clients.
//! State is stored in `actions_log` (request `id` = `actions_log.id`; while `decision` is empty the request is open); the
//! request fields live in `args_json.req`. Waiting dispatchers are in memory: after a core crash open requests are closed
//! as `deny` (`recover`).

mod decide;
mod model;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::watch;

use crate::{
    Action, Audit, ChatStore, Clock, NewAction, Notifier, PermEngine, PermError, PermResult,
    SettingsStore, Subject,
};

pub use decide::{DecideError, DecideInput, DecisionKind, RuleInput};
pub use model::{Outcome, PermissionRequest, Status};

/// Default request lifetime, seconds (10 minutes).
pub const DEFAULT_TTL_SECS: i64 = 600;

/// What is needed to open a request.
#[derive(Debug, Clone)]
pub struct RequestSpec {
    pub subject: Subject,
    pub action: Action,
    pub target: Option<String>,
    pub args: Option<serde_json::Value>,
    /// `policy` or `observer`.
    pub source: &'static str,
    pub kind: Option<String>,
    pub verdict: Option<String>,
    pub detail: Option<String>,
    pub origin: Option<String>,
    pub grant_token: Option<i64>,
}

pub(crate) struct Waiter {
    pub tx: watch::Sender<Option<Outcome>>,
    pub session_id: Option<i64>,
    pub expires_at: i64,
}

pub(crate) struct Inner {
    pub audit: Audit,
    pub chat: Arc<dyn ChatStore>,
    pub notifier: Arc<dyn Notifier>,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<dyn SettingsStore>,
    pub engine: PermEngine,
    pub waiters: Mutex<HashMap<i64, Waiter>>,
    /// Approved HTTP-client requests: (token, action, target) → until when valid.
    pub grants: Mutex<HashMap<(i64, Action, String), i64>>,
    /// Requests are closed one at a time: "Always" writes a rule only after seeing the request open under this
    /// lock, and closes it before leaving the lock. A losing decision leaves no rule.
    pub decide_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct PermissionService {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) fn topic(session_id: i64) -> String {
    format!("session.{session_id}")
}

impl PermissionService {
    #[must_use]
    pub fn new(
        audit: Audit,
        chat: Arc<dyn ChatStore>,
        notifier: Arc<dyn Notifier>,
        clock: Arc<dyn Clock>,
        settings: Arc<dyn SettingsStore>,
        engine: PermEngine,
    ) -> PermissionService {
        PermissionService {
            inner: Arc::new(Inner {
                audit,
                chat,
                notifier,
                clock,
                settings,
                engine,
                waiters: Mutex::new(HashMap::new()),
                grants: Mutex::new(HashMap::new()),
                decide_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Lifetime of a new request: `ask_ttl_min` of the `permissions` page (minutes), otherwise 10 minutes.
    pub async fn ttl_secs(&self) -> i64 {
        match self.inner.settings.get("permissions").await {
            Ok(d) => d
                .doc
                .get("ask_ttl_min")
                .and_then(serde_json::Value::as_i64)
                .map_or(DEFAULT_TTL_SECS, |m| m * 60),
            Err(_) => DEFAULT_TTL_SECS,
        }
    }

    /// Opens a request: a log entry without a decision, a chat card, `message.created` and `permission.request` frames.
    pub async fn open(&self, spec: RequestSpec) -> PermResult<PermissionRequest> {
        let inner = &self.inner;
        let now = inner.clock.now_unix();
        let expires_at = now + self.ttl_secs().await;
        let level = if spec.source == "observer" {
            "warn"
        } else {
            "ask"
        };
        let req_meta = json!({
            "source": spec.source, "level": level, "kind": spec.kind, "verdict": spec.verdict,
            "detail": spec.detail, "expires_at": expires_at, "origin": spec.origin,
            "grant": spec.grant_token,
        });
        let entry = NewAction {
            actor: spec.subject.actor,
            session_id: spec.subject.session_id,
            persona_id: spec.subject.persona_id,
            action: spec.action.as_str().to_string(),
            target: spec.target.clone(),
            args: Some(json!({"args": spec.args, "req": req_meta})),
            decision: None,
            result: None,
        };
        let id = inner.audit.record(entry).await?;
        let row = inner
            .audit
            .store()
            .get_action(id)
            .await?
            .ok_or(PermError::Unavailable)?;
        let req = PermissionRequest::from_row(&row).ok_or(PermError::Unavailable)?;
        let (tx, _rx) = watch::channel(None);
        inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Waiter {
                    tx,
                    session_id: spec.subject.session_id,
                    expires_at,
                },
            );
        if let Some(sid) = req.session_id {
            match inner
                .chat
                .create_system(sid, req.card_text(), req.card_meta())
                .await
            {
                Ok(card) => {
                    let tp = topic(sid);
                    inner
                        .notifier
                        .publish_event(&tp, json!({"kind": "message.created", "message": card}));
                    inner.notifier.publish_event(
                        &tp,
                        json!({"kind": "permission.request", "request": req.to_json()}),
                    );
                }
                Err(_) => {
                    // The session was deleted while the request was opening: nobody is waiting for a decision.
                    self.close_one(id, "session_deleted").await;
                }
            }
        }
        Ok(req)
    }

    /// Waits for the outcome of request `id`. A closed service or unknown request is a `closed` denial.
    pub async fn wait(&self, id: i64) -> Outcome {
        let rx = {
            let w = self.inner.waiters.lock().unwrap_or_else(|e| e.into_inner());
            w.get(&id).map(|w| w.tx.subscribe())
        };
        let Some(mut rx) = rx else {
            return self.settled(id).await;
        };
        loop {
            if let Some(o) = rx.borrow_and_update().clone() {
                return o;
            }
            if rx.changed().await.is_err() {
                return self.settled(id).await;
            }
        }
    }

    /// Outcome of an already closed request, from the log.
    async fn settled(&self, id: i64) -> Outcome {
        let row = self.inner.audit.store().get_action(id).await.ok().flatten();
        match row
            .and_then(|r| PermissionRequest::from_row(&r))
            .map(|r| r.status)
        {
            Some(Status::Allowed) => Outcome::Allowed,
            Some(Status::Expired) => Outcome::Denied("expired"),
            Some(Status::Denied) => Outcome::Denied("denied"),
            _ => Outcome::Denied("closed"),
        }
    }

    /// Current state of a request; an expired open request is closed right here (`expired`).
    pub async fn get(&self, id: i64) -> PermResult<Option<PermissionRequest>> {
        let Some(row) = self.inner.audit.store().get_action(id).await? else {
            return Ok(None);
        };
        let Some(req) = PermissionRequest::from_row(&row) else {
            return Ok(None);
        };
        if req.status == Status::Pending
            && req
                .expires_at
                .is_some_and(|e| e <= self.inner.clock.now_unix())
        {
            self.expire(id).await;
            let row = self.inner.audit.store().get_action(id).await?;
            return Ok(row.and_then(|r| PermissionRequest::from_row(&r)));
        }
        Ok(Some(req))
    }

    /// Previously approved request of this token: the grant is one-time.
    #[must_use]
    pub fn take_grant(&self, token_id: i64, action: Action, target: &str) -> bool {
        let now = self.inner.clock.now_unix();
        let mut g = self.inner.grants.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|_, until| *until > now);
        g.remove(&(token_id, action, target.to_string())).is_some()
    }
}
