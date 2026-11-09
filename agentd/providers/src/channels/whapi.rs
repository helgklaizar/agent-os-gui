//! Whapi (WhatsApp, unofficial channel): `GET /health`, `POST /messages/text`, `GET /messages/list`.
//! The channel token comes from the secret store (`account.<id>`) and never reaches logs or errors. The `QR` status in `/health` means
//! "number unlinked": the code must be scanned in the Whapi dashboard. The `messages/list` reply shapes are an assumption.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentd_core::{
    Capability, ChannelAccount, ChannelProvider, ConnectKind, Flow, FlowStatus, Health, Incoming,
    PortFuture, SecretStore, SendOutcome,
};
use serde_json::{Value, json};

use crate::HttpClient;

pub const WHAPI_BASE_URL: &str = "https://gate.whapi.cloud";

pub struct Whapi {
    http: HttpClient,
    secrets: Arc<dyn SecretStore>,
    base: String,
    /// `time_from` of the next poll per account (seconds).
    since: Mutex<HashMap<i64, i64>>,
}

impl Whapi {
    #[must_use]
    pub fn new(http: HttpClient, secrets: Arc<dyn SecretStore>, base: &str) -> Whapi {
        Whapi {
            http,
            secrets,
            base: base.trim_end_matches('/').to_string(),
            since: Mutex::new(HashMap::new()),
        }
    }

    /// `None` means no token; `Err` is a network error. Otherwise status and JSON.
    async fn call(
        &self,
        acc: &ChannelAccount,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Option<Result<(u16, Value), ()>> {
        let token = self.secrets.get(&format!("account.{}", acc.id)).await?;
        let headers = [
            ("authorization", format!("Bearer {}", token.expose())),
            ("content-type", "application/json".to_string()),
            ("accept", "application/json".to_string()),
        ];
        let raw = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        let url = format!("{}{path}", self.base);
        Some(
            match self.http.call_json(method, &url, &headers, raw).await {
                Ok((st, bytes)) => Ok((st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))),
                Err(_) => Err(()),
            },
        )
    }
}

fn status_text(v: &Value) -> String {
    v["status"]["text"]
        .as_str()
        .unwrap_or("")
        .to_ascii_uppercase()
}

impl ChannelProvider for Whapi {
    fn kind(&self) -> &'static str {
        "wa"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Send, Capability::Receive]
    }

    fn connect_kind(&self) -> ConnectKind {
        ConnectKind::Qr
    }

    fn connect<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Flow> {
        Box::pin(async move {
            let status = match self.call(acc, "GET", "/health", None).await {
                None => FlowStatus::WaitingUser,
                Some(Ok((200, v))) => match status_text(&v).as_str() {
                    "AUTH" => FlowStatus::Connected,
                    "QR" => FlowStatus::WaitingUser,
                    _ => FlowStatus::Failed,
                },
                Some(_) => FlowStatus::Failed,
            };
            Flow::new(ConnectKind::Qr, status)
        })
    }

    fn health<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Health> {
        Box::pin(async move {
            match self.call(acc, "GET", "/health", None).await {
                Some(Ok((200, v))) => match status_text(&v).as_str() {
                    "AUTH" => Health::Ok,
                    "QR" => Health::Down,
                    _ => Health::Degraded,
                },
                Some(Ok(_)) => Health::Degraded,
                Some(Err(())) | None => Health::Down,
            }
        })
    }

    fn send<'a>(
        &'a self,
        acc: &'a ChannelAccount,
        to: &'a str,
        text: &'a str,
        _key: &'a str,
    ) -> PortFuture<'a, SendOutcome> {
        Box::pin(async move {
            let body = json!({"to": to, "body": text});
            match self.call(acc, "POST", "/messages/text", Some(body)).await {
                None => SendOutcome::Rejected("no_token"),
                Some(Ok((200, v))) if v["sent"] == Value::Bool(true) => SendOutcome::Delivered {
                    ext_id: v["message"]["id"].as_str().map(str::to_string),
                },
                Some(Ok((400, _))) => SendOutcome::Rejected("bad_request"),
                Some(Ok((401, _))) => SendOutcome::Rejected("unauthorized"),
                Some(Ok((403, _))) => SendOutcome::Rejected("forbidden"),
                // Rate limit, failure, drop, unparseable reply: delivery may have happened, retrying is unsafe.
                Some(_) => SendOutcome::Unknown,
            }
        })
    }

    fn poll<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Vec<Incoming>> {
        Box::pin(async move {
            let from = self
                .since
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&acc.id)
                .copied();
            let path = from.map_or("/messages/list?count=50".to_string(), |t| {
                format!("/messages/list?count=50&time_from={t}")
            });
            let Some(Ok((200, v))) = self.call(acc, "GET", &path, None).await else {
                return Vec::new();
            };
            let list = v["messages"].as_array().cloned().unwrap_or_default();
            if let Some(max) = list.iter().filter_map(|m| m["timestamp"].as_i64()).max() {
                self.since
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(acc.id, max + 1);
            }
            list.iter()
                .filter(|m| m["from_me"] != Value::Bool(true) && m["type"] == "text")
                .filter_map(|m| {
                    Some(Incoming {
                        ext_id: m["id"].as_str()?.to_string(),
                        handle: m["from"].as_str()?.to_string(),
                        body: m["text"]["body"]
                            .as_str()
                            .filter(|t| !t.trim().is_empty())?
                            .to_string(),
                    })
                })
                .collect()
        })
    }
}
