//! `PolicyGate`: permissions + observer + permission request. The single point every agent action
//! and every `agent` and `module` token mutation passes through.

use std::sync::Arc;

use serde_json::Value;

use crate::{
    Action, Actor, Audit, Decision, Finding, Hit, Mode, NewAction, Observer, ObserverStatus,
    PermEngine, PermResult, PermissionRequest, PermissionService, RequestSpec,
};

/// Who acts and on whose behalf.
#[derive(Debug, Clone, Copy)]
pub struct Subject {
    pub actor: Actor,
    pub session_id: Option<i64>,
    pub persona_id: Option<i64>,
}

/// The action under check.
#[derive(Debug, Clone)]
pub struct GateInput {
    pub subject: Subject,
    pub action: Action,
    pub target: Option<String>,
    pub args: Option<Value>,
    /// Source of untrusted text during whose processing the action arose ("from the page", "from an email").
    pub untrusted: Option<String>,
    /// HTTP client token: after the request is approved, the same request of this token passes once (`202` and a retry).
    pub grant_token: Option<i64>,
}

/// Check outcome.
#[derive(Debug)]
pub enum Gate {
    /// May be executed; `action_id` is the log entry with `decision = allow`.
    Allow { action_id: i64 },
    /// A user decision is needed: the request is open, wait with `PermissionService::wait`.
    Ask(Box<PermissionRequest>),
    /// Forbidden: `reason` is `policy_deny` or `observer_blocked`.
    Deny {
        action_id: i64,
        reason: &'static str,
    },
}

#[derive(Clone)]
pub struct PolicyGate {
    engine: PermEngine,
    observer: Arc<dyn Observer>,
    audit: Audit,
    requests: PermissionService,
}

impl PolicyGate {
    #[must_use]
    pub fn new(
        engine: PermEngine,
        observer: Arc<dyn Observer>,
        audit: Audit,
        requests: PermissionService,
    ) -> PolicyGate {
        PolicyGate {
            engine,
            observer,
            audit,
            requests,
        }
    }

    #[must_use]
    pub fn requests(&self) -> &PermissionService {
        &self.requests
    }

    #[must_use]
    pub fn audit(&self) -> &Audit {
        &self.audit
    }

    #[must_use]
    pub fn observer(&self) -> &Arc<dyn Observer> {
        &self.observer
    }

    async fn closed(
        &self,
        input: &GateInput,
        decision: Decision,
        result: Option<&str>,
    ) -> PermResult<i64> {
        self.audit
            .record(NewAction {
                actor: input.subject.actor,
                session_id: input.subject.session_id,
                persona_id: input.subject.persona_id,
                action: input.action.as_str().to_string(),
                target: input.target.clone(),
                args: input.args.clone(),
                decision: Some(decision),
                result: result.map(str::to_string),
            })
            .await
    }

    async fn finding(&self, hit: &Hit, ref_id: i64, verdict: &'static str) {
        let f = Finding {
            kind: hit.kind,
            severity: hit.severity,
            ref_table: Some("actions_log"),
            ref_id: Some(ref_id.to_string()),
            verdict,
            detail: Some(format!("rule={}", hit.rule)),
        };
        if self.audit.finding(f).await.is_err() {
            tracing::warn!(
                scope = "observer",
                event = "finding_lost",
                "finding not recorded"
            );
        }
    }

    /// Action check: effective mode, observer, untrusted context.
    pub async fn check(&self, input: GateInput) -> PermResult<Gate> {
        let mode = self
            .engine
            .effective(
                input.subject.persona_id,
                input.subject.session_id,
                input.action,
            )
            .await?;
        if mode == Some(Mode::Deny) {
            let id = self
                .closed(&input, Decision::Deny, Some("policy_deny"))
                .await?;
            return Ok(Gate::Deny {
                action_id: id,
                reason: "policy_deny",
            });
        }
        let mut hit = input
            .target
            .as_deref()
            .and_then(|t| self.observer.check_action(input.action, t));
        // Rules are the first line; the observer model (if any) looks only at what the rules let through.
        if hit.is_none()
            && let Some(t) = input.target.as_deref()
        {
            hit = self.observer.classify(input.action, t).await;
        }
        if let Some(h) = hit.as_ref().filter(|h| h.verdict == "blocked") {
            let id = self
                .closed(&input, Decision::Deny, Some("observer_blocked"))
                .await?;
            self.finding(h, id, "blocked").await;
            return Ok(Gate::Deny {
                action_id: id,
                reason: "observer_blocked",
            });
        }
        let unavailable =
            self.observer.status() == ObserverStatus::Unavailable && input.action.is_dangerous();
        let by_observer = hit.is_some() || unavailable;
        let untrusted_ask = input.untrusted.is_some() && input.action != Action::Read;
        let ask = mode == Some(Mode::Ask) || by_observer || untrusted_ask;
        if !ask {
            let id = self.closed(&input, Decision::Allow, None).await;
            return id.map(|action_id| Gate::Allow { action_id });
        }
        // An approved request of the same token for the same action and target passes once (HTTP: `202`, then a retry).
        if let Some(token) = input.grant_token
            && !by_observer
            && self
                .requests
                .take_grant(token, input.action, input.target.as_deref().unwrap_or(""))
        {
            let id = self.closed(&input, Decision::Allow, Some("grant")).await?;
            return Ok(Gate::Allow { action_id: id });
        }
        let detail = match (&hit, unavailable) {
            (Some(h), _) => Some(format!("observer: rule {}", h.rule)),
            (None, true) => Some("observer unavailable".to_string()),
            (None, false) => None,
        };
        let spec = RequestSpec {
            subject: input.subject,
            action: input.action,
            target: input.target.clone(),
            args: input.args.clone(),
            source: if by_observer { "observer" } else { "policy" },
            kind: hit.as_ref().map(|h| h.kind.to_string()),
            verdict: by_observer.then(|| "asked".to_string()),
            detail,
            origin: input.untrusted.clone(),
            grant_token: input.grant_token,
        };
        let req = self.requests.open(spec).await?;
        if let Some(h) = &hit {
            self.finding(h, req.id, "asked").await;
        }
        Ok(Gate::Ask(Box::new(req)))
    }
}
