//! Closing requests: user decision ("Always" creates a rule), TTL expiry, session deletion, restart.

use std::time::Duration;

use serde_json::{Value, json};

use crate::requests::{Outcome, PermissionRequest, PermissionService, Status, topic};
use crate::{Actor, Decision, Level, Mode, NewAction, PermError, PermResult, PutError, Rule};

/// The "Always" rule: persona level or global (`session` is not allowed), default mode `allow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleInput {
    pub level: Level,
    pub scope_id: i64,
    pub mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Allow,
    Deny,
    /// Allow and save the rule. A rule mode of `deny` also rejects the current request.
    Always(RuleInput),
}

#[derive(Debug, Clone)]
pub struct DecideInput {
    pub kind: DecisionKind,
    /// Who decided: token label (`gui:3`).
    pub decided_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecideError {
    NotFound,
    /// The request is already decided: the first decision wins (`409`), current status.
    AlreadyDecided(Status),
    /// The "Always" rule would weaken a higher level (`403`); the request stays open.
    Loosen,
    /// The persona has no such rule.
    RuleScopeNotFound,
    Unavailable,
}

impl From<PermError> for DecideError {
    fn from(_: PermError) -> DecideError {
        DecideError::Unavailable
    }
}

impl PermissionService {
    /// User decision on an open request. First-wins: the loser gets `AlreadyDecided`.
    pub async fn decide(
        &self,
        id: i64,
        input: DecideInput,
    ) -> Result<PermissionRequest, DecideError> {
        let _one_at_a_time = self.inner.decide_lock.lock().await;
        let req = self.get(id).await?.ok_or(DecideError::NotFound)?;
        if req.status != Status::Pending {
            return Err(DecideError::AlreadyDecided(req.status));
        }
        let (decision, result, outcome) = match input.kind {
            DecisionKind::Allow => (Decision::AskApproved, "allowed", Outcome::Allowed),
            DecisionKind::Deny => (Decision::AskDenied, "denied", Outcome::Denied("denied")),
            DecisionKind::Always(rule) => {
                let r = Rule {
                    level: rule.level,
                    scope_id: rule.scope_id,
                    action: req.action,
                    mode: rule.mode,
                };
                match self.inner.engine.put(r, &input.decided_by).await {
                    Ok(_) => {}
                    Err(PutError::Loosen) => return Err(DecideError::Loosen),
                    Err(PutError::NotFound) => return Err(DecideError::RuleScopeNotFound),
                    Err(PutError::Unavailable) => return Err(DecideError::Unavailable),
                }
                self.log_rule(&r, id).await;
                if rule.mode == Mode::Deny {
                    (Decision::AskDenied, "denied", Outcome::Denied("denied"))
                } else {
                    (Decision::AskApproved, "always", Outcome::Allowed)
                }
            }
        };
        if !self
            .finish(id, decision, result, outcome, &input.decided_by)
            .await?
        {
            let cur = self.get(id).await?.map_or(Status::Denied, |r| r.status);
            return Err(DecideError::AlreadyDecided(cur));
        }
        self.get(id).await?.ok_or(DecideError::NotFound)
    }

    async fn log_rule(&self, r: &Rule, request_id: i64) {
        let entry = NewAction {
            actor: Actor::User,
            session_id: None,
            persona_id: None,
            action: "permission_put".into(),
            target: Some(format!("{}/{}/{}", r.level.as_str(), r.scope_id, r.action)),
            args: Some(json!({"mode": r.mode.as_str(), "via": "always", "request_id": request_id})),
            decision: None,
            result: Some("ok".into()),
        };
        if self.inner.audit.record(entry).await.is_err() {
            tracing::error!(
                scope = "perms",
                event = "audit_failed",
                "the \"Always\" rule was not written to the log"
            );
        }
    }

    /// Records the decision (if the request is still open), wakes the waiter, updates the card and broadcasts
    /// `permission.resolved`. `false` — the request is already closed by another.
    pub(crate) async fn finish(
        &self,
        id: i64,
        decision: Decision,
        result: &str,
        outcome: Outcome,
        decided_by: &str,
    ) -> PermResult<bool> {
        let inner = &self.inner;
        let Some(row) = inner.audit.store().get_action(id).await? else {
            return Ok(false);
        };
        let now = inner.clock.now_unix();
        let mut doc: Value = row
            .args_json
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or(Value::Null);
        // Decision fields go into `req`; if the arguments are already cleared (session deleted), only the decision is written.
        let grant_token = doc["req"]["grant"].as_i64();
        let args = doc
            .get_mut("req")
            .and_then(Value::as_object_mut)
            .map(|req| {
                req.insert("decided_by".into(), decided_by.into());
                req.insert("decided_at".into(), now.into());
            });
        let new_args = args.map(|()| doc.to_string());
        if !inner
            .audit
            .store()
            .decide_action(id, decision, Some(result.to_string()), new_args)
            .await?
        {
            return Ok(false);
        }
        if decision == Decision::AskApproved
            && let (Some(token), Some(action)) = (grant_token, crate::Action::parse(&row.action))
        {
            let until = now + self.ttl_secs().await;
            inner
                .grants
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    (token, action, row.target.clone().unwrap_or_default()),
                    until,
                );
        }
        let waiter = inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(w) = waiter {
            // The receiver may be gone (dispatcher left): the decision is already recorded.
            drop(w.tx.send(Some(outcome)));
        }
        if let Some(sid) = row.session_id {
            let after = inner.audit.store().get_action(id).await?;
            if let Some(req) = after.and_then(|r| PermissionRequest::from_row(&r)) {
                let patch = json!({"request": req.to_json(), "status": req.status.as_str()});
                // The card may be missing: the session was deleted together with its messages.
                drop(inner.chat.update_card(id, patch).await);
                inner.notifier.publish_event(
                    &topic(sid),
                    json!({"kind": "permission.resolved", "request_id": id,
                           "status": req.status.as_str(), "decided_by": decided_by, "result": result}),
                );
            }
        }
        Ok(true)
    }

    /// Expired request: `deny`, result `expired`.
    pub(crate) async fn expire(&self, id: i64) {
        self.close_one(id, "expired").await;
    }

    /// Closes the request with a denial for `reason` (`expired`, `session_deleted`, `core_restart`).
    pub(crate) async fn close_one(&self, id: i64, reason: &'static str) {
        let by = if reason == "expired" {
            "timeout"
        } else {
            "system"
        };
        let _one_at_a_time = self.inner.decide_lock.lock().await;
        if self
            .finish(id, Decision::Deny, reason, Outcome::Denied(reason), by)
            .await
            .is_err()
        {
            tracing::warn!(
                scope = "perms",
                event = "close_failed",
                reason,
                "request not closed"
            );
        }
    }

    /// Stopping a turn (`stop`): the open request is closed with the denial `cancelled`.
    pub async fn cancel(&self, id: i64) {
        self.close_one(id, "cancelled").await;
    }

    /// Closes requests whose time has run out.
    pub async fn expire_due(&self) {
        let now = self.inner.clock.now_unix();
        let due: Vec<i64> = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, w)| w.expires_at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.expire(id).await;
        }
    }

    /// Session deletion: its open requests are closed as `deny`. Called before the row is deleted.
    pub async fn close_session(&self, session_id: i64) {
        let Ok(rows) = self
            .inner
            .audit
            .store()
            .pending_actions(Some(session_id))
            .await
        else {
            tracing::warn!(
                scope = "perms",
                event = "close_session_failed",
                "open requests not read"
            );
            return;
        };
        for r in rows {
            self.close_one(r.id, "session_deleted").await;
        }
        let ids: Vec<i64> = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, w)| w.session_id == Some(session_id))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.close_one(id, "session_deleted").await;
        }
    }

    /// Core start: requests left open after a crash are closed with the denial `core_restart`; nobody is waiting for them.
    pub async fn recover(&self) -> PermResult<usize> {
        let rows = self.inner.audit.store().pending_actions(None).await?;
        let n = rows.len();
        for r in rows {
            self.close_one(r.id, "core_restart").await;
        }
        Ok(n)
    }

    /// Background expiry task: every 250 ms by the core clock.
    pub fn spawn_sweeper(&self) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                me.expire_due().await;
            }
        })
    }
}
