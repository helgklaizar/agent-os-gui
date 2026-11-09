//! Chat calls: messages, stop, attachments, permissions, journal. Responses are returned as JSON, parsed by the controllers.

use serde_json::{Value, json};

use crate::client::HttpClient;
use crate::error::ClientError;
use crate::http::{RawResponse, Request};

/// Outcome of `POST /v1/sessions/{id}/messages` (api.md section 5.2).
#[derive(Debug, Clone, PartialEq)]
pub enum SendOutcome {
    /// `201`: message created.
    Created(Value),
    /// `200`: repeat of a `client_id`, the core returned the already stored message.
    Duplicate(Value),
    /// `202`: action in `ask` mode, the body is a `PermissionRequest`.
    NeedsDecision(Value),
}

impl SendOutcome {
    pub fn from_response(resp: &RawResponse) -> Option<SendOutcome> {
        let body: Value = resp.json().ok()?;
        match resp.status {
            201 => Some(SendOutcome::Created(body)),
            200 => Some(SendOutcome::Duplicate(body)),
            202 => Some(SendOutcome::NeedsDecision(body)),
            _ => None,
        }
    }
}

fn protocol(what: &str) -> ClientError {
    ClientError::Protocol(format!("{what}: unexpected response"))
}

impl HttpClient {
    /// `GET /v1/sessions/{id}/messages`: a history page, newest to oldest.
    pub async fn list_messages(
        &self,
        session_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Value>, ClientError> {
        let mut path = format!("/v1/sessions/{session_id}/messages?limit={limit}");
        if let Some(b) = before {
            path.push_str(&format!("&before={b}"));
        }
        self.call(Request::get(&path)).await?.json()
    }

    /// `POST /v1/sessions/{id}/messages`: `201` created, `200` repeat of a `client_id`, `202` a decision is needed.
    pub async fn send_message(
        &self,
        session_id: i64,
        client_id: &str,
        body_md: &str,
        attachment_ids: &[i64],
    ) -> Result<SendOutcome, ClientError> {
        let mut body = json!({ "body_md": body_md, "client_id": client_id });
        if !attachment_ids.is_empty() {
            body["attachment_ids"] = json!(attachment_ids);
        }
        let path = format!("/v1/sessions/{session_id}/messages");
        let resp = self.call(Request::post(&path, body)).await?;
        SendOutcome::from_response(&resp).ok_or_else(|| protocol("sending message"))
    }

    /// `POST /v1/sessions/{id}/stop`.
    pub async fn stop_session(&self, session_id: i64) -> Result<(), ClientError> {
        let path = format!("/v1/sessions/{session_id}/stop");
        self.call(Request::post(&path, json!({}))).await?;
        Ok(())
    }

    /// `POST /v1/sessions/{id}/attachments`: file upload, the response is an `Attachment`.
    pub async fn upload_attachment(
        &self,
        session_id: i64,
        filename: &str,
        mime: &str,
        data: &[u8],
    ) -> Result<Value, ClientError> {
        let path = format!("/v1/sessions/{session_id}/attachments");
        self.call(Request::multipart(&path, filename, mime, data))
            .await?
            .json()
    }

    /// `POST /v1/permission-requests/{id}/decision`: `decision` is `allow`, `deny` or `always` (then a `rule` is needed).
    pub async fn decide_permission(
        &self,
        request_id: i64,
        decision: &str,
        rule: Option<Value>,
    ) -> Result<Value, ClientError> {
        let mut body = json!({ "decision": decision });
        if let Some(r) = rule {
            body["rule"] = r;
        }
        let path = format!("/v1/permission-requests/{request_id}/decision");
        self.call(Request::post(&path, body)).await?.json()
    }

    /// `GET /v1/permission-requests/{id}`.
    pub async fn permission_request(&self, request_id: i64) -> Result<Value, ClientError> {
        self.call(Request::get(&format!(
            "/v1/permission-requests/{request_id}"
        )))
        .await?
        .json()
    }

    /// `GET /v1/permissions?scope&scope_id`.
    pub async fn list_permissions(
        &self,
        scope: &str,
        scope_id: i64,
    ) -> Result<Vec<Value>, ClientError> {
        let path = format!("/v1/permissions?scope={scope}&scope_id={scope_id}");
        self.call(Request::get(&path)).await?.json()
    }

    /// `GET /v1/actions?session_id=…` (action journal): a page `{items, next_cursor}`.
    pub async fn list_actions(&self, session_id: Option<i64>) -> Result<Value, ClientError> {
        let path = session_id.map_or_else(
            || "/v1/actions".to_string(),
            |s| format!("/v1/actions?session_id={s}"),
        );
        self.call(Request::get(&path)).await?.json()
    }

    /// `PUT /v1/permissions/{scope}/{scope_id}/{action}` (admin, local only): set `mode`.
    pub async fn put_permission(
        &self,
        scope: &str,
        scope_id: i64,
        action: &str,
        mode: &str,
    ) -> Result<Value, ClientError> {
        let path = format!("/v1/permissions/{scope}/{scope_id}/{action}");
        self.call(Request::put(&path, json!({ "mode": mode })))
            .await?
            .json()
    }

    /// `GET /v1/observer-findings?kind=…`: a page `{items, next_cursor}`.
    pub async fn list_findings(&self, kind: Option<&str>) -> Result<Value, ClientError> {
        let path = kind.map_or_else(
            || "/v1/observer-findings".to_string(),
            |k| format!("/v1/observer-findings?kind={k}"),
        );
        self.call(Request::get(&path)).await?.json()
    }

    /// `GET /v1/permissions` without a filter: all explicit rules (for choosing the level in "Always").
    pub async fn list_all_permissions(&self) -> Result<Value, ClientError> {
        self.call(Request::get("/v1/permissions")).await?.json()
    }
}
