//! Helpers for chat end-to-end tests: `scripted` account, messages, waiting for a reply, local "provider".
#![allow(dead_code)]

use std::time::Duration;

use core_client::api::Frame;
use serde_json::{Value, json};

use super::{Env, Reply};

/// `session.<id>` event by its `kind` field.
pub fn kind_of(f: &Frame) -> &str {
    f.data["kind"].as_str().unwrap_or("")
}

pub fn is_done(f: &Frame) -> bool {
    kind_of(f) == "message.done"
}

impl Env {
    /// A `scripted` model account, the `scripted` model on the `models` page and a session with that model: `(account_id, session_id)`.
    pub async fn model_session(&self) -> (i64, i64) {
        let acc = self
            .admin
            .make("/accounts", json!({"type": "scripted", "label": "test"}))
            .await["id"]
            .as_i64()
            .unwrap();
        let (_, page) = self.admin.get("/settings/models").await;
        let rev = page["rev"].as_i64().unwrap_or(0);
        let (st, b) = self
            .admin
            .patch_if_match(
                "/settings/models",
                json!({"accounts": {acc.to_string(): {"model": "scripted"}}}),
                rev,
            )
            .await;
        assert_eq!(st, 200, "{b}");
        let s = self.chat.make("/sessions", json!({"name": "chat"})).await["id"]
            .as_i64()
            .unwrap();
        let (st, b) = self
            .chat
            .patch(&format!("/sessions/{s}"), json!({"cfg": {"model": acc}}))
            .await;
        assert_eq!(st, 200, "{b}");
        (acc, s)
    }

    pub async fn say(&self, session: i64, client_id: &str, text: &str) -> Reply {
        self.chat
            .post(
                &format!("/sessions/{session}/messages"),
                json!({"body_md": text, "client_id": client_id}),
            )
            .await
    }

    pub async fn messages(&self, session: i64) -> Vec<Value> {
        let (st, all) = self
            .read
            .get(&format!("/sessions/{session}/messages?limit=200"))
            .await;
        assert_eq!(st, 200);
        let mut v = all.as_array().unwrap().clone();
        v.reverse();
        v
    }

    /// Waits until an agent message appears in the session (stream running).
    pub async fn wait_agent_done_or_streaming(&self, session: i64) {
        for _ in 0..200 {
            let m = self.messages(session).await;
            if m.iter()
                .any(|x| x["role"] == "agent" && !x["body_md"].as_str().unwrap_or("").is_empty())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("reply did not start");
    }

    /// Waits until the session has no agent messages in `streaming` and the agent message count is `n`.
    pub async fn wait_agent_done(&self, session: i64, n: usize) -> Vec<Value> {
        for _ in 0..400 {
            let m = self.messages(session).await;
            let agent: Vec<&Value> = m.iter().filter(|x| x["role"] == "agent").collect();
            if agent.len() >= n && agent.iter().all(|x| x["status"] != "streaming") {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("agent reply did not finish");
    }
}

impl Env {
    /// Waits for a permission request card in the session: `(request_id, card)`.
    pub async fn wait_card(&self, session: i64) -> (i64, Value) {
        for _ in 0..200 {
            let m = self.messages(session).await;
            let found = m
                .iter()
                .find(|m| m["role"] == "system" && m["meta"]["kind"] == "permission_request");
            if let Some(c) = found {
                return (c["meta"]["request_id"].as_i64().unwrap(), c.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("permission request card did not appear");
    }

    /// Owner decision (`gui`/`admin`) on a request.
    pub async fn decide(&self, id: i64, body: Value) -> Reply {
        self.admin
            .post(&format!("/permission-requests/{id}/decision"), body)
            .await
    }

    /// Log rows by action: `(decision, result)`; inspecting the DB restarts the core.
    pub async fn log_rows(
        &mut self,
        action: &'static str,
    ) -> Vec<(Option<String>, Option<String>)> {
        self.inspect(move |c| {
            c.prepare("SELECT decision, result FROM actions_log WHERE action = ?1 ORDER BY id")?
                .query_map([action], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .await
    }
}

/// Local "provider": answers any request with an empty reply and the given status (key and address check).
pub async fn status_server(status: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                drop(s.read(&mut buf).await);
                let body = r#"{"error":"secret-detail-from-provider"}"#;
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                drop(s.write_all(resp.as_bytes()).await);
            });
        }
    });
    url
}

impl Env {
    /// Account `status` value; waits until it stops being `disconnected`.
    pub async fn account_status_settled(&self, id: i64) -> String {
        for _ in 0..100 {
            let (_, list) = self.read.get("/accounts").await;
            let st = list
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == id)
                .map(|a| a["status"].as_str().unwrap().to_string())
                .unwrap();
            if st != "disconnected" {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        "disconnected".into()
    }

    pub async fn patch_models(&self, patch: Value) -> Reply {
        let (_, page) = self.admin.get("/settings/models").await;
        self.admin
            .patch_if_match("/settings/models", patch, page["rev"].as_i64().unwrap_or(0))
            .await
    }
}
