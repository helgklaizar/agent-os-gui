//! Shared by the settings tests: the mock (including a restart on the same DB), socket calls, `If-Match`.
#![allow(dead_code)]

#[path = "../contract/rawhttp.rs"]
mod rawhttp;

use std::path::Path;

use fake_agentd::{Config, FakeAgentd, Kind, Scope, TokenSpec};
use serde_json::{Value, json};

pub use rawhttp::{Req, Resp, send};

pub struct Rig {
    pub d: FakeAgentd,
    pub admin: String,
}

pub async fn rig() -> Rig {
    start(Config::temp().no_ticker().manual_clock()).await
}

/// Mock on a given DB: a second start on the same file is a "core restart".
pub async fn rig_db(db: &Path) -> Rig {
    start(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .db(db.to_path_buf()),
    )
    .await
}

pub async fn start(config: Config) -> Rig {
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

    /// Call with `If-Match: "<rev>"`.
    pub async fn call_if(&self, method: &str, path: &str, rev: i64, body: Value) -> Resp {
        let req = Req::new(method, path)
            .token(&self.admin)
            .header("If-Match", &format!("\"{rev}\""))
            .json(body);
        send(self.d.socket_path(), req).await
    }

    pub async fn get(&self, path: &str) -> Value {
        let r = self.call("GET", path, None).await;
        assert_eq!(r.status, 200, "GET {path}: {}", text(&r));
        r.json()
    }

    pub async fn rev_of(&self, path: &str) -> i64 {
        self.get(path).await["rev"].as_i64().unwrap_or(-1)
    }

    /// `PATCH /v1/settings/<page>` with the current `rev`.
    pub async fn patch_page(&self, page: &str, body: Value) -> Resp {
        let path = format!("/v1/settings/{page}");
        let rev = self.rev_of(&path).await;
        self.call_if("PATCH", &path, rev, body).await
    }

    pub fn token(&self, scope: Scope, kind: Kind) -> String {
        match self.d.issue(TokenSpec::new(scope, kind)) {
            Ok(t) => t.token,
            Err(e) => panic!("token not issued: {e}"),
        }
    }
}

pub fn text(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

/// List of field names from `error.details.fields` of a 422 response.
pub fn fields(r: &Resp) -> Vec<String> {
    r.json()["error"]["details"]["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

pub fn code(r: &Resp) -> String {
    r.json()["error"]["code"].as_str().unwrap_or("").to_string()
}

pub async fn new_account(r: &Rig, kind: &str, label: &str) -> i64 {
    let w = r
        .call(
            "POST",
            "/v1/accounts",
            Some(json!({"type": kind, "label": label})),
        )
        .await;
    assert_eq!(w.status, 201, "{}", text(&w));
    w.json()["id"].as_i64().unwrap_or(-1)
}

pub async fn new_persona(r: &Rig, body: Value) -> Value {
    let w = r.call("POST", "/v1/personas", Some(body)).await;
    assert_eq!(w.status, 201, "{}", text(&w));
    w.json()
}

pub fn manifest(id: &str) -> Value {
    json!({"id": id, "name": "Custom", "version": "1.0.0", "entry": "index.html", "placement": "panel",
           "permissions": ["GET /sessions"], "network": ["api.example.com"], "fs": ["~/notes"], "questions": []})
}
