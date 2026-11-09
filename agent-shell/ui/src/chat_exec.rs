//! Execution of chat commands: the decision on a permission request. No GTK; called by a worker thread or a test.

use controllers::chat_types::{AttachmentRef, ReqStatus};
use controllers::permissions::{Mode, PermDecision, RuleScope, RuleSpec};
use core_client::{ClientError, HttpClient};
use serde_json::{Value, json};

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Allow => "allow",
        Mode::Ask => "ask",
        Mode::Deny => "deny",
    }
}

/// `rule` body for `decision = always` (api.md `PermissionDecision`).
pub fn rule_json(rule: RuleSpec) -> Value {
    let scope = match rule.scope {
        RuleScope::Persona => "persona",
        RuleScope::Global => "global",
    };
    json!({ "scope": scope, "scope_id": rule.scope_id, "mode": mode_name(rule.mode) })
}

pub fn decision_name(d: PermDecision) -> &'static str {
    match d {
        PermDecision::Allow => "allow",
        PermDecision::Deny => "deny",
        PermDecision::Always => "always",
    }
}

/// `POST /permission-requests/{id}/decision`; the core's reply (request state) is not needed here, the `permission.resolved` frame provides it.
pub async fn decide(
    http: &HttpClient,
    request_id: i64,
    decision: PermDecision,
    rule: Option<RuleSpec>,
) -> Result<(), ClientError> {
    http.decide_permission(request_id, decision_name(decision), rule.map(rule_json))
        .await
        .map(drop)
}

/// Decision outcome for the controller: `409` means another client decided (the status is taken from the core), anything else did not get through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionOutcome {
    Done,
    Rejected(ReqStatus),
    Failed,
}

/// Decision on a permission request with `409 conflict` handling (first wins, api.md section 5.4).
pub async fn decide_outcome(
    http: &HttpClient,
    request_id: i64,
    decision: PermDecision,
    rule: Option<RuleSpec>,
) -> DecisionOutcome {
    match decide(http, request_id, decision, rule).await {
        Ok(()) => DecisionOutcome::Done,
        Err(e) if e.api().is_some_and(|a| a.status == 409) => {
            let status = http
                .permission_request(request_id)
                .await
                .ok()
                .and_then(|r| r["status"].as_str().and_then(ReqStatus::parse));
            status.map_or(DecisionOutcome::Failed, DecisionOutcome::Rejected)
        }
        Err(_) => DecisionOutcome::Failed,
    }
}

/// Uploads attachments before sending: `POST /sessions/{id}/attachments`, returns the `id`s in order.
/// The content is read here, the controller does not see it; an error is a reason without paths or content.
pub async fn upload_all(
    http: &HttpClient,
    session_id: i64,
    files: &[AttachmentRef],
) -> Result<Vec<i64>, String> {
    let mut ids = Vec::new();
    for f in files {
        let data = std::fs::read(&f.path).map_err(|_| "attachment file not read".to_string())?;
        let att = http
            .upload_attachment(session_id, &f.name, "application/octet-stream", &data)
            .await
            .map_err(|e| e.to_string())?;
        ids.push(att["id"].as_i64().ok_or("attachment response has no id")?);
    }
    Ok(ids)
}
