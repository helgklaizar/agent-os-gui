//! Issue, list and revoke tokens over HTTP: real `TokenStore`, DB and `agentd-api`.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use agentd_api::{Server, ServerConfig, Services};
use agentd_core::{Scope, TokenKind};
use common::{Env, Resp, T0, env, http, new_token};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct Rig {
    env: Env,
    server: Server,
    admin: String,
}

async fn rig(tcp: bool) -> Rig {
    let env = env();
    let admin = env
        .store
        .issue(new_token(TokenKind::Gui, Scope::Admin))
        .await
        .unwrap()
        .value
        .expose()
        .clone();
    let mut cfg = ServerConfig::new(env.dir.path().join("agentd.sock"));
    cfg.tcp_addr = "127.0.0.1:0".parse().unwrap();
    cfg.tcp_enabled = tcp;
    cfg.clock = env.clock.clone();
    let server = Server::start_booting(cfg).await.unwrap();
    let mut services = Services::new(env.store.clone());
    services.tokens = Some(env.store.clone());
    services.status = Some(env.db.clone());
    server.set_ready(services);
    Rig { env, server, admin }
}

fn bearer(t: &str) -> Vec<String> {
    vec![format!("Authorization: Bearer {t}")]
}

impl Rig {
    async fn req(&self, method: &str, path: &str, token: &str, body: Option<Value>) -> Resp {
        let mut s = tokio::net::UnixStream::connect(self.server.socket_path())
            .await
            .unwrap();
        let mut extra = bearer(token);
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        if !payload.is_empty() {
            extra.push("Content-Type: application/json".into());
            extra.push(format!("Content-Length: {}", payload.len()));
        }
        let mut resp = common::http_body(&mut s, method, path, "localhost", &extra, &payload).await;
        resp.take().expect("response")
    }

    async fn issue(&self, name: &str, scope: &str) -> Value {
        let r = self
            .req(
                "POST",
                "/v1/tokens",
                &self.admin,
                Some(json!({"name": name, "scope": scope})),
            )
            .await;
        assert_eq!(r.status, 201, "{}", r.body);
        r.json()
    }

    async fn addr(&self) -> SocketAddr {
        for _ in 0..300 {
            if let Some(a) = self.server.tcp().local_addr() {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("TCP did not come up");
    }
}

fn code(r: &Resp) -> String {
    r.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn token_issue_shown_once() {
    let rig = rig(false).await;
    let issued = rig.issue("phone", "chat").await;
    let value = issued["token"]
        .as_str()
        .expect("value in the issue response")
        .to_string();
    assert!(
        value.starts_with("agt_") && value.len() == 47,
        "format agt_ + 43 characters"
    );
    assert_eq!(issued["name"], "phone");
    assert_eq!(issued["kind"], "external");
    assert_eq!(issued["scope"], "chat");
    assert_eq!(issued["revoked"], false);
    assert_eq!(issued["expires_at"], T0 + 30 * 86_400, "30 days by default");
    // The issued token works.
    let ping = rig.req("GET", "/v1/settings/theme", &value, None).await;
    assert_ne!(ping.status, 401);
    // The list does not contain the value.
    let list = rig.req("GET", "/v1/tokens", &rig.admin, None).await;
    assert_eq!(list.status, 200);
    assert!(
        !list.body.contains(&value),
        "token value is not in the list"
    );
    assert!(
        !list.body.contains("token_hash") && !list.body.contains("\"token\""),
        "{}",
        list.body
    );
    let items = list.json();
    assert_eq!(items.as_array().unwrap().len(), 2, "gui and external");
    // The value is in no DB file (including WAL) nor anywhere else in the data directory.
    for entry in walk(rig.env.dir.path()) {
        if entry.extension().is_some_and(|e| e == "sock") {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !bytes.windows(value.len()).any(|w| w == value.as_bytes()),
            "raw value found in {}",
            entry.display()
        );
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.is_file() {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn token_list_hides_hash_and_shows_every_kind() {
    let rig = rig(false).await;
    let mut agent = new_token(TokenKind::Agent, Scope::Chat);
    agent.allowlist = Some(vec!["GET /sessions".into()]);
    rig.env.store.issue(agent).await.unwrap();
    let mut module = new_token(TokenKind::Module, Scope::Read);
    module.allowlist = Some(vec![]);
    rig.env.store.issue(module).await.unwrap();
    rig.issue("cli", "read").await;
    let list = rig.req("GET", "/v1/tokens", &rig.admin, None).await.json();
    let kinds: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    for k in ["gui", "agent", "module", "external"] {
        assert!(kinds.contains(&k), "{kinds:?}");
    }
    let agent = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "agent")
        .unwrap();
    assert_eq!(agent["allowlist"], json!(["GET /sessions"]));
    let gui = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "gui")
        .unwrap();
    assert!(gui["expires_at"].is_null());
    assert!(
        gui["last_used"].as_i64().is_some(),
        "last_used is set after the admin's first request"
    );
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|t| t.get("token_hash").is_none() && t.get("token").is_none())
    );
}

#[tokio::test]
async fn external_scope_admin_rejected_422() {
    let rig = rig(false).await;
    for body in [
        json!({"name": "x", "scope": "admin"}),
        json!({"name": "x", "scope": "root"}),
        json!({"name": "", "scope": "read"}),
        json!({"name": "x"}),
        json!({"name": "x", "scope": "read", "ttl_days": 0}),
        json!({"name": "x", "scope": "read", "ttl_days": 91}),
        json!({"name": "x", "scope": "read", "ttl_days": "30"}),
        json!({"name": "x", "scope": "read", "kind": "gui"}),
        json!({"name": "x", "scope": "read", "kind": "agent"}),
    ] {
        let r = rig
            .req("POST", "/v1/tokens", &rig.admin, Some(body.clone()))
            .await;
        assert_eq!(
            (r.status, code(&r).as_str()),
            (422, "validation_failed"),
            "{body}"
        );
        assert!(
            r.json()["error"]["details"]["fields"][0]["field"].is_string(),
            "{body}"
        );
    }
    let n: i64 = rig
        .env
        .db
        .read(|c| c.query_row("SELECT count(*) FROM api_token", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 1, "nothing but the admin token was created");
    let ok = rig
        .req(
            "POST",
            "/v1/tokens",
            &rig.admin,
            Some(json!({"name": "x", "scope": "read", "ttl_days": 90})),
        )
        .await;
    assert_eq!(ok.status, 201);
    assert_eq!(ok.json()["expires_at"], T0 + 90 * 86_400);
    let bad = rig.req("POST", "/v1/tokens", &rig.admin, None).await;
    assert_eq!((bad.status, code(&bad).as_str()), (400, "bad_request"));
}

#[tokio::test]
async fn created_by_is_the_issuing_token() {
    let rig = rig(false).await;
    let t = rig.issue("x", "read").await;
    let admin_id: i64 = rig
        .env
        .db
        .read(|c| {
            c.query_row("SELECT id FROM api_token WHERE kind='gui'", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(t["created_by"], admin_id);
}

#[tokio::test]
async fn token_routes_local_only() {
    let rig = rig(true).await;
    let addr = rig.addr().await;
    for (m, path) in [
        ("GET", "/v1/tokens"),
        ("POST", "/v1/tokens"),
        ("DELETE", "/v1/tokens/1"),
    ] {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let r = http(&mut s, m, path, &addr.to_string(), &bearer(&rig.admin))
            .await
            .unwrap();
        assert_eq!(
            (r.status, code(&r).as_str()),
            (403, "insufficient_scope"),
            "{m} {path}"
        );
    }
    let n: i64 = rig
        .env
        .db
        .read(|c| c.query_row("SELECT count(*) FROM api_token", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 1, "nothing issued over TCP");
}

#[tokio::test]
async fn token_scopes_for_token_routes() {
    let rig = rig(false).await;
    let chat = rig.issue("c", "chat").await["token"]
        .as_str()
        .unwrap()
        .to_string();
    for (m, p) in [
        ("GET", "/v1/tokens"),
        ("POST", "/v1/tokens"),
        ("DELETE", "/v1/tokens/1"),
    ] {
        let r = rig.req(m, p, &chat, None).await;
        assert_eq!(
            (r.status, code(&r).as_str()),
            (403, "insufficient_scope"),
            "{m} {p}"
        );
    }
}

#[tokio::test]
async fn token_revoke_closes_ws_4401() {
    let rig = rig(false).await;
    let issued = rig.issue("phone", "read").await;
    let (id, value) = (
        issued["id"].as_i64().unwrap(),
        issued["token"].as_str().unwrap().to_string(),
    );
    let stream = tokio::net::UnixStream::connect(rig.server.socket_path())
        .await
        .unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {value}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    let other = rig.issue("second", "read").await["token"]
        .as_str()
        .unwrap()
        .to_string();
    let stream2 = tokio::net::UnixStream::connect(rig.server.socket_path())
        .await
        .unwrap();
    let mut req2 = "ws://localhost/v1/ws".into_client_request().unwrap();
    req2.headers_mut()
        .insert("authorization", format!("Bearer {other}").parse().unwrap());
    let (mut ws2, _) = tokio_tungstenite::client_async(req2, stream2)
        .await
        .unwrap();

    let del = rig
        .req("DELETE", &format!("/v1/tokens/{id}"), &rig.admin, None)
        .await;
    assert_eq!(del.status, 204, "{}", del.body);
    let code = loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("close arrived")
        {
            Some(Ok(Message::Close(Some(f)))) => break u16::from(f.code),
            Some(Ok(_)) => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(code, 4401);
    // A revoked token is rejected immediately; another connection stays alive.
    assert_eq!(
        rig.req("GET", "/v1/settings/theme", &value, None)
            .await
            .status,
        401
    );
    let still = tokio::time::timeout(Duration::from_millis(300), ws2.next()).await;
    assert!(still.is_err(), "another token's connection is not closed");
    // Repeat revoke is a conflict, unknown id is 404, garbage is 400.
    let again = rig
        .req("DELETE", &format!("/v1/tokens/{id}"), &rig.admin, None)
        .await;
    assert_eq!(
        (again.status, again.json()["error"]["code"].clone()),
        (409, json!("conflict"))
    );
    assert_eq!(
        again.json()["error"]["details"]["reason"],
        "already_revoked"
    );
    assert_eq!(
        rig.req("DELETE", "/v1/tokens/99999", &rig.admin, None)
            .await
            .status,
        404
    );
    assert_eq!(
        rig.req("DELETE", "/v1/tokens/abc", &rig.admin, None)
            .await
            .status,
        400
    );
}

#[tokio::test]
async fn admin_can_revoke_agent_and_module_tokens() {
    let rig = rig(false).await;
    let mut agent = new_token(TokenKind::Agent, Scope::Chat);
    agent.allowlist = Some(vec!["GET /sessions".into()]);
    let issued = rig.env.store.issue(agent).await.unwrap();
    let del = rig
        .req(
            "DELETE",
            &format!("/v1/tokens/{}", issued.id),
            &rig.admin,
            None,
        )
        .await;
    assert_eq!(del.status, 204);
    assert_eq!(
        rig.req("GET", "/v1/sessions", issued.value.expose(), None)
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn oversized_body_is_413() {
    let rig = rig(false).await;
    let big = json!({"name": "x".repeat(2 * 1024 * 1024), "scope": "read"});
    let r = rig.req("POST", "/v1/tokens", &rig.admin, Some(big)).await;
    assert_eq!((r.status, code(&r).as_str()), (413, "payload_too_large"));
}
