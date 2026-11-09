//! Telegram Bot API: connect by token (`PUT /secrets/account.<id>`), receive via `getUpdates`, send via
//! `sendMessage`. The token is read from the secret store at request time and never reaches logs or errors.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentd_core::{
    Capability, ChannelAccount, ChannelProvider, ConnectKind, Flow, FlowStatus, Health, Incoming,
    PortFuture, SecretStore, SendOutcome,
};
use serde_json::{Value, json};

use crate::HttpClient;

pub const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

pub struct Telegram {
    http: HttpClient,
    secrets: Arc<dyn SecretStore>,
    base: String,
    /// Next `offset` for `getUpdates` per account.
    offsets: Mutex<HashMap<i64, i64>>,
}

impl Telegram {
    #[must_use]
    pub fn new(http: HttpClient, secrets: Arc<dyn SecretStore>, base: &str) -> Telegram {
        Telegram {
            http,
            secrets,
            base: base.trim_end_matches('/').to_string(),
            offsets: Mutex::new(HashMap::new()),
        }
    }

    /// Bot API method call: `(status, JSON body)`; `None` means no token. A network error is `Err`.
    async fn call(
        &self,
        acc: &ChannelAccount,
        method: &str,
        body: &Value,
    ) -> Option<Result<(u16, Value), ()>> {
        let token = self.secrets.get(&format!("account.{}", acc.id)).await?;
        let url = format!("{}/bot{}/{method}", self.base, token.expose());
        let raw = serde_json::to_vec(body).unwrap_or_default();
        Some(match self.http.post_json(&url, raw).await {
            Ok((status, bytes)) => Ok((
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )),
            Err(_) => Err(()),
        })
    }
}

fn incoming(update: &Value) -> Option<Incoming> {
    let m = &update["message"];
    let body = m["text"].as_str().filter(|t| !t.trim().is_empty())?;
    let handle = m["from"]["username"]
        .as_str()
        .map(|u| format!("@{u}"))
        .or_else(|| m["chat"]["id"].as_i64().map(|i| i.to_string()))?;
    Some(Incoming {
        ext_id: update["update_id"].as_i64()?.to_string(),
        handle,
        body: body.to_string(),
    })
}

impl ChannelProvider for Telegram {
    fn kind(&self) -> &'static str {
        "tg"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Send, Capability::Receive]
    }

    fn connect_kind(&self) -> ConnectKind {
        ConnectKind::Token
    }

    fn connect<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Flow> {
        Box::pin(async move {
            let status = match self.call(acc, "getMe", &json!({})).await {
                None => FlowStatus::WaitingUser,
                Some(Ok((200, v))) if v["ok"] == Value::Bool(true) => FlowStatus::Connected,
                Some(_) => FlowStatus::Failed,
            };
            Flow::new(ConnectKind::Token, status)
        })
    }

    fn health<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Health> {
        Box::pin(async move {
            match self.call(acc, "getMe", &json!({})).await {
                Some(Ok((200, _))) => Health::Ok,
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
            let body = json!({"chat_id": to, "text": text});
            match self.call(acc, "sendMessage", &body).await {
                None => SendOutcome::Rejected("no_token"),
                Some(Ok((200, v))) if v["ok"] == Value::Bool(true) => SendOutcome::Delivered {
                    ext_id: v["result"]["message_id"].as_i64().map(|i| i.to_string()),
                },
                // Confirmed Telegram rejection: the message was definitely not delivered.
                Some(Ok((400, _))) => SendOutcome::Rejected("bad_request"),
                Some(Ok((401, _))) => SendOutcome::Rejected("unauthorized"),
                Some(Ok((403, _))) => SendOutcome::Rejected("forbidden"),
                // Rate limit, server failure, drop, unparseable reply: delivery may have happened, retrying is unsafe.
                Some(_) => SendOutcome::Unknown,
            }
        })
    }

    fn poll<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Vec<Incoming>> {
        Box::pin(async move {
            let offset = self
                .offsets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&acc.id)
                .copied()
                .unwrap_or(0);
            let body = json!({"offset": offset, "timeout": 0, "allowed_updates": ["message"]});
            let Some(Ok((200, v))) = self.call(acc, "getUpdates", &body).await else {
                return Vec::new();
            };
            let updates = v["result"].as_array().cloned().unwrap_or_default();
            if let Some(max) = updates.iter().filter_map(|u| u["update_id"].as_i64()).max() {
                self.offsets
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(acc.id, max + 1);
            }
            updates.iter().filter_map(incoming).collect()
        })
    }
}
