//! Notification feed calls: `GET /events`, mark as read, subtype registry. Responses are returned as JSON,
//! parsed by the controllers (`controllers::notif_types`).

use serde_json::{Value, json};

use crate::client::HttpClient;
use crate::error::ClientError;
use crate::http::Request;

/// Parameters of `GET /v1/events` (api.md section 5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventQuery {
    pub kind: Option<String>,
    pub sub: Option<String>,
    pub unread: Option<bool>,
    pub session_id: Option<i64>,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

impl EventQuery {
    /// Path with a query string; values are encoded, an empty query is `/v1/events`.
    pub fn path(&self) -> String {
        let mut pairs: Vec<String> = Vec::new();
        if let Some(k) = &self.kind {
            pairs.push(format!("kind={}", encode(k)));
        }
        if let Some(s) = &self.sub {
            pairs.push(format!("sub={}", encode(s)));
        }
        if let Some(u) = self.unread {
            pairs.push(format!("unread={u}"));
        }
        if let Some(s) = self.session_id {
            pairs.push(format!("session_id={s}"));
        }
        if let Some(b) = self.before {
            pairs.push(format!("before={b}"));
        }
        if let Some(l) = self.limit {
            pairs.push(format!("limit={l}"));
        }
        if pairs.is_empty() {
            "/v1/events".to_string()
        } else {
            format!("/v1/events?{}", pairs.join("&"))
        }
    }
}

/// Body of `POST /v1/events/read`: the fields that are set go into the body, empty means "all unread".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsRead {
    pub ids: Option<Vec<i64>>,
    pub kind: Option<String>,
    pub sub: Option<String>,
    pub session_id: Option<i64>,
}

impl EventsRead {
    pub fn body(&self) -> Value {
        let mut m = serde_json::Map::new();
        if let Some(ids) = &self.ids {
            m.insert("ids".into(), json!(ids));
        }
        if let Some(k) = &self.kind {
            m.insert("kind".into(), json!(k));
        }
        if let Some(s) = &self.sub {
            m.insert("sub".into(), json!(s));
        }
        if let Some(s) = self.session_id {
            m.insert("session_id".into(), json!(s));
        }
        Value::Object(m)
    }
}

/// Percent-encoding of a query value: only `A-Z a-z 0-9 - . _ ~` stay as is.
fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl HttpClient {
    /// `GET /v1/events`: `{ items, counts }`.
    pub async fn list_events(&self, query: &EventQuery) -> Result<Value, ClientError> {
        self.call(Request::get(&query.path())).await?.json()
    }

    /// `POST /v1/events/{id}/read`.
    pub async fn read_event(&self, id: i64) -> Result<(), ClientError> {
        let path = format!("/v1/events/{id}/read");
        self.call(Request::post(&path, json!({}))).await?;
        Ok(())
    }

    /// `POST /v1/events/read`: how many events were marked.
    pub async fn read_events(&self, filter: &EventsRead) -> Result<i64, ClientError> {
        let resp = self
            .call(Request::post("/v1/events/read", filter.body()))
            .await?;
        let body: Value = resp.json()?;
        body.get("marked")
            .and_then(Value::as_i64)
            .ok_or_else(|| ClientError::Protocol("marking events: unexpected response".into()))
    }

    /// `GET /v1/events/subtypes`.
    pub async fn event_subtypes(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/events/subtypes")).await?.json()
    }
}
