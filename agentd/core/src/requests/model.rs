//! Permission request (`PermissionRequest`, `docs/reference/api.md` 6): state, schema JSON and parsing of an `actions_log` row.

use serde_json::{Value, json};

use crate::{Action, ActionRow, Decision};

/// Request status: `pending → allowed | denied | expired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Allowed,
    Denied,
    Expired,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Allowed => "allowed",
            Status::Denied => "denied",
            Status::Expired => "expired",
        }
    }
}

/// Permission request: id equals `actions_log.id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: i64,
    pub session_id: Option<i64>,
    pub action: Action,
    pub target: Option<String>,
    /// `policy` or `observer`.
    pub source: String,
    /// `ask` or `warn` (observer card).
    pub level: String,
    /// Observer finding type.
    pub kind: Option<String>,
    /// Observer verdict.
    pub verdict: Option<String>,
    pub status: Status,
    pub detail: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    /// Source of the untrusted text that caused the action ("from the page"); not part of the schema, lives
    /// in the card `meta`.
    pub origin: Option<String>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

impl PermissionRequest {
    /// Request from a log row. `None` — the entry is not a permission request (no saved request fields: a plain entry or
    /// arguments cleared by session deletion).
    #[must_use]
    pub fn from_row(row: &ActionRow) -> Option<PermissionRequest> {
        let doc: Value = serde_json::from_str(row.args_json.as_deref()?).ok()?;
        let req = doc.get("req")?;
        let status = match (row.decision, row.result.as_deref()) {
            (None, _) => Status::Pending,
            (Some(Decision::AskApproved | Decision::Allow), _) => Status::Allowed,
            (Some(Decision::Deny), Some("expired")) => Status::Expired,
            (Some(Decision::AskDenied | Decision::Deny), _) => Status::Denied,
        };
        Some(PermissionRequest {
            id: row.id,
            session_id: row.session_id,
            action: Action::parse(&row.action)?,
            target: row.target.clone(),
            source: s(req, "source")?,
            level: s(req, "level")?,
            kind: s(req, "kind"),
            verdict: s(req, "verdict"),
            status,
            detail: s(req, "detail"),
            decided_by: s(req, "decided_by"),
            decided_at: req.get("decided_at").and_then(Value::as_i64),
            expires_at: req.get("expires_at").and_then(Value::as_i64),
            created_at: row.ts,
            origin: s(req, "origin"),
        })
    }

    /// `PermissionRequest` schema from openapi-draft.yaml.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "session_id": self.session_id,
            "action": self.action.as_str(),
            "target": self.target,
            "source": self.source,
            "level": self.level,
            "kind": self.kind,
            "verdict": self.verdict,
            "status": self.status.as_str(),
            "detail": self.detail,
            "decided_by": self.decided_by,
            "decided_at": self.decided_at,
            "expires_at": self.expires_at,
            "created_at": self.created_at,
        })
    }

    /// Chat card `meta`: `kind = permission_request`, `request_id`, the full `request` and untrusted
    /// source markers.
    #[must_use]
    pub fn card_meta(&self) -> Value {
        json!({
            "kind": "permission_request",
            "request_id": self.id,
            "request": self.to_json(),
            "untrusted": self.origin.is_some(),
            "origin": self.origin,
        })
    }

    /// Card text (escaped on output).
    #[must_use]
    pub fn card_text(&self) -> String {
        let target = self.target.as_deref().unwrap_or("");
        let cut: String = target.chars().take(200).collect();
        format!("Permission request: {} — {cut}", self.action)
    }
}

/// How a pending operation ended: what the caller (dispatcher) receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    /// Denial: `denied`, `expired`, `session_deleted`, `closed`.
    Denied(&'static str),
}
