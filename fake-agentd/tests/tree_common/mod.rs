//! Shared by the tree API tests: the mock, socket calls with an admin token, helpers for creating folders and sessions.
#![allow(dead_code)]

#[path = "../contract/rawhttp.rs"]
pub mod rawhttp;

use fake_agentd::{Config, FakeAgentd};
use rawhttp::{Req, Resp, send};
use serde_json::{Value, json};

pub struct Rig {
    pub d: FakeAgentd,
    pub admin: String,
}

pub async fn rig() -> Rig {
    let d = match FakeAgentd::start(Config::temp().no_ticker()).await {
        Ok(d) => d,
        Err(e) => panic!("fake-agentd did not start: {e}"),
    };
    let admin = d.admin_token().to_string();
    Rig { d, admin }
}

impl Rig {
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        let mut req = Req::new(method, path).token(&self.admin);
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

    pub async fn post(&self, path: &str, body: Value) -> Resp {
        self.call("POST", path, Some(body)).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Resp {
        self.call("PATCH", path, Some(body)).await
    }

    pub async fn created(&self, path: &str, body: Value) -> Value {
        let r = self.post(path, body).await;
        assert_eq!(
            r.status,
            201,
            "POST {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        r.json()
    }

    pub async fn folder(&self, name: &str, parent: Option<i64>, persona: Option<i64>) -> i64 {
        let f = self
            .created(
                "/v1/folders",
                json!({"name": name, "parent_id": parent, "persona_id": persona}),
            )
            .await;
        f["id"].as_i64().unwrap_or(-1)
    }

    pub async fn session(&self, name: &str, folder: Option<i64>) -> i64 {
        let s = self
            .created("/v1/sessions", json!({"name": name, "folder_id": folder}))
            .await;
        s["id"].as_i64().unwrap_or(-1)
    }

    pub async fn session_json(&self, id: i64) -> Value {
        self.get(&format!("/v1/sessions/{id}")).await
    }

    pub async fn persona_id(&self, name: &str) -> i64 {
        let list = self.get("/v1/personas").await;
        list.as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == name)
            .and_then(|p| p["id"].as_i64())
            .unwrap_or(-1)
    }
}

pub fn code(r: &Resp) -> String {
    r.json()["error"]["code"].as_str().unwrap_or("").to_string()
}

pub fn by_name<'a>(list: &'a Value, name: &str) -> &'a Value {
    list.as_array()
        .into_iter()
        .flatten()
        .find(|x| x["name"] == name)
        .unwrap_or(&Value::Null)
}
