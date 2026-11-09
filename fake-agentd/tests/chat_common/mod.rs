//! Shared by the chat tests: the mock, socket calls, WS frame observer.
#![allow(dead_code)]

#[path = "../contract/rawhttp.rs"]
mod rawhttp;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fake_agentd::{Config, FakeAgentd, Kind, Scope, TokenSpec};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub use rawhttp::{Req, Resp, send};

pub const UUID_A: &str = "0192f1a0-0000-7000-8000-00000000000a";
pub const UUID_B: &str = "0192f1a0-0000-7000-8000-00000000000b";

pub struct Rig {
    pub d: FakeAgentd,
    pub admin: String,
}

pub async fn rig() -> Rig {
    rig_with(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .reply_delay(Duration::from_millis(5)),
    )
    .await
}

pub async fn rig_with(config: Config) -> Rig {
    let d = match FakeAgentd::start(config).await {
        Ok(d) => d,
        Err(e) => panic!("fake-agentd did not start: {e}"),
    };
    let admin = d.admin_token().to_string();
    Rig { d, admin }
}

impl Rig {
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        self.call_as(&self.admin, method, path, body).await
    }

    pub async fn call_as(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Resp {
        let mut req = Req::new(method, path).token(token);
        if let Some(b) = body {
            req = req.json(b);
        }
        send(self.d.socket_path(), req).await
    }

    pub async fn get(&self, path: &str) -> Value {
        let r = self.call("GET", path, None).await;
        assert_eq!(
            r.status,
            200,
            "GET {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        r.json()
    }

    pub fn token(&self, scope: Scope, kind: Kind) -> String {
        match self.d.issue(TokenSpec::new(scope, kind)) {
            Ok(t) => t.token,
            Err(e) => panic!("token not issued: {e}"),
        }
    }

    /// First session from the fixtures (`Release 1.2`) and its id.
    pub async fn session(&self) -> i64 {
        let r = self
            .call("POST", "/v1/sessions", Some(json!({"name": "Chat"})))
            .await;
        assert_eq!(r.status, 201);
        r.json()["id"].as_i64().unwrap_or(-1)
    }

    pub async fn say(&self, session: i64, text: &str, client_id: &str) -> Resp {
        let body = json!({"body_md": text, "client_id": client_id});
        self.call(
            "POST",
            &format!("/v1/sessions/{session}/messages"),
            Some(body),
        )
        .await
    }

    pub async fn messages(&self, session: i64) -> Vec<Value> {
        let v = self
            .get(&format!("/v1/sessions/{session}/messages?limit=200"))
            .await;
        v.as_array().cloned().unwrap_or_default()
    }

    /// Sets the session persona and does a `PUT` rule as admin.
    pub async fn put_rule(&self, scope: &str, id: i64, action: &str, mode: &str) -> Resp {
        self.call(
            "PUT",
            &format!("/v1/permissions/{scope}/{id}/{action}"),
            Some(json!({"mode": mode})),
        )
        .await
    }

    pub async fn decide(&self, request: i64, body: Value) -> Resp {
        self.call(
            "POST",
            &format!("/v1/permission-requests/{request}/decision"),
            Some(body),
        )
        .await
    }

    pub fn log_rows(&self, sql: &str) -> Vec<Vec<Option<String>>> {
        let conn = self.d.db().reader().expect("reader");
        let mut st = conn.prepare(sql).expect("query");
        let n = st.column_count();
        st.query_map([], |r| {
            (0..n)
                .map(|i| {
                    r.get::<_, rusqlite::types::Value>(i).map(|v| match v {
                        rusqlite::types::Value::Null => None,
                        rusqlite::types::Value::Integer(i) => Some(i.to_string()),
                        rusqlite::types::Value::Text(t) => Some(t),
                        other => Some(format!("{other:?}")),
                    })
                })
                .collect()
        })
        .expect("rows")
        .flatten()
        .collect()
    }
}

/// Frames of one topic that arrived over WS.
pub struct Watch {
    frames: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn watch(d: &FakeAgentd, token: &str, topic: &str) -> Watch {
    let stream = UnixStream::connect(d.socket_path()).await.expect("socket");
    let mut req = "ws://localhost/v1/ws"
        .into_client_request()
        .expect("request");
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().expect("header"),
    );
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream)
        .await
        .expect("WS");
    ws.send(Message::text(
        json!({"op": "subscribe", "topic": topic}).to_string(),
    ))
    .await
    .expect("subscription");
    let frames = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    let task = tokio::spawn(async move {
        while let Some(Ok(m)) = ws.next().await {
            if let Message::Text(t) = m
                && let Ok(v) = serde_json::from_str::<Value>(t.as_str())
            {
                sink.lock().unwrap_or_else(|e| e.into_inner()).push(v);
            }
        }
    });
    Watch { frames, task }
}

impl Watch {
    pub fn all(&self) -> Vec<Value> {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Events of `kind` (`data.kind`) in arrival order.
    pub fn events(&self, kind: &str) -> Vec<Value> {
        self.all()
            .into_iter()
            .filter(|f| f["data"]["kind"] == kind)
            .map(|f| f["data"].clone())
            .collect()
    }

    pub fn snapshot(&self) -> Value {
        self.all()
            .into_iter()
            .find(|f| f["type"] == "snapshot")
            .map(|f| f["data"].clone())
            .unwrap_or(Value::Null)
    }

    /// Waits until `ready` returns a value, up to `secs` seconds.
    pub async fn until<T>(&self, secs: u64, ready: impl Fn(&Watch) -> Option<T>) -> Option<T> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Some(v) = ready(self) {
                return Some(v);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }

    pub async fn event(&self, kind: &str, secs: u64) -> Option<Value> {
        self.until(secs, |w| w.events(kind).into_iter().next())
            .await
    }

    pub async fn done(&self, secs: u64) -> Option<Value> {
        self.event("message.done", secs).await
    }
}

pub fn multipart(name: &str, mime: &str, content: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----fabound";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

pub fn code(r: &Resp) -> String {
    r.json()["error"]["code"].as_str().unwrap_or("").to_string()
}

/// Waits for an open permission request and returns it.
pub async fn pending(r: &Rig) -> Value {
    for _ in 0..500 {
        if let Some(p) = r.d.pending_requests().into_iter().next() {
            return p;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("permission request did not appear");
}

/// Sends a command message and waits for the permission request.
pub async fn ask(r: &Rig, s: i64, text: &str) -> Value {
    r.say(s, text, UUID_A).await;
    pending(r).await
}

/// Agent action log rows by action.
pub fn rows(r: &Rig, action: &str) -> Vec<Vec<Option<String>>> {
    r.log_rows(&format!(
        "SELECT decision, result, reversible, actor FROM actions_log WHERE action = '{action}' AND actor = 'agent' ORDER BY id"
    ))
}
