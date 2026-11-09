//! Web token with the `@decide` grant: issue, lifetime, permission limits over TCP; real `TokenStore` and API.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use agentd_api::{Server, ServerConfig, Services};
use agentd_core::{Scope, TokenKind};
use common::{Env, Resp, T0, env, http, http_body, new_token};
use serde_json::{Value, json};

struct Rig {
    env: Env,
    server: Server,
    admin: String,
}

async fn rig() -> Rig {
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
    cfg.tcp_enabled = true;
    cfg.clock = env.clock.clone();
    let server = Server::start_booting(cfg).await.unwrap();
    let mut services = Services::new(env.store.clone());
    services.tokens = Some(env.store.clone());
    services.status = Some(env.db.clone());
    server.set_ready(services);
    Rig { env, server, admin }
}

impl Rig {
    async fn local(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        let mut s = tokio::net::UnixStream::connect(self.server.socket_path())
            .await
            .unwrap();
        let mut extra = vec![format!("Authorization: Bearer {}", self.admin)];
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        if !payload.is_empty() {
            extra.push("Content-Type: application/json".into());
            extra.push(format!("Content-Length: {}", payload.len()));
        }
        http_body(&mut s, method, path, "localhost", &extra, &payload)
            .await
            .expect("response")
    }

    async fn tcp(&self, method: &str, path: &str, token: &str) -> Resp {
        let addr = self.addr().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let extra = vec![
            format!("Authorization: Bearer {token}"),
            "Content-Type: application/json".into(),
            "Content-Length: 2".into(),
        ];
        http_body(&mut s, method, path, &addr.to_string(), &extra, "{}")
            .await
            .expect("response")
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

    async fn issue(&self, body: Value) -> Resp {
        self.local("POST", "/v1/tokens", Some(body)).await
    }
}

fn code(r: &Resp) -> String {
    r.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn web_token_defaults_and_listing() {
    let r = rig().await;
    let resp = r
        .issue(json!({"name": "browser", "scope": "chat", "decide": true}))
        .await;
    assert_eq!(resp.status, 201, "{}", resp.body);
    let doc = resp.json();
    assert_eq!(doc["scope"], "decide");
    assert_eq!(doc["kind"], "web");
    assert_eq!(
        doc["expires_at"],
        T0 + 7 * 86_400,
        "default lifetime is 7 days"
    );
    assert!(doc["token"].as_str().unwrap().starts_with("agt_"));
    let list = r.local("GET", "/v1/tokens", None).await.json();
    let web = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "browser")
        .unwrap();
    assert_eq!(
        (web["kind"].as_str(), web["scope"].as_str()),
        (Some("web"), Some("decide"))
    );
    assert_eq!(web["allowlist"], Value::Null);
    assert!(web.get("token").is_none() && web.get("token_hash").is_none());
}

#[tokio::test]
async fn web_token_limits() {
    let r = rig().await;
    let ok = r
        .issue(json!({"name": "a", "scope": "chat", "decide": true, "ttl_days": 30}))
        .await;
    assert_eq!(ok.status, 201, "{}", ok.body);
    for body in [
        json!({"name": "b", "scope": "chat", "decide": true, "ttl_days": 31}),
        json!({"name": "c", "scope": "chat", "decide": true, "ttl_days": 0}),
        json!({"name": "d", "scope": "read", "decide": true}),
        json!({"name": "e", "scope": "admin", "decide": true}),
        json!({"name": "", "scope": "chat", "decide": true}),
    ] {
        let x = r.issue(body.clone()).await;
        assert_eq!(x.status, 422, "{body}: {}", x.body);
    }
    // Without `decide` an external token stays a plain external `chat`.
    let plain = r.issue(json!({"name": "p", "scope": "chat"})).await.json();
    assert_eq!(
        (plain["kind"].as_str(), plain["scope"].as_str()),
        (Some("external"), Some("chat"))
    );
}

#[tokio::test]
async fn web_token_over_tcp_boundaries() {
    let r = rig().await;
    let web = r
        .issue(json!({"name": "w", "scope": "chat", "decide": true}))
        .await
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let plain = r.issue(json!({"name": "p", "scope": "chat"})).await.json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let decision = "/v1/permission-requests/999/decision";
    // A plain external token cannot decide over TCP.
    let denied = r.tcp("POST", decision, &plain).await;
    assert_eq!(denied.status, 403, "{}", denied.body);
    // The web token passes the access layer: next is `404`/`503`, but not `403` or `401`.
    let passed = r.tcp("POST", decision, &web).await;
    assert!(
        ![401, 403].contains(&passed.status),
        "{} {}",
        passed.status,
        passed.body
    );
    // The rest of `admin` and "local only" is closed to it.
    for (m, p) in [
        ("POST", "/v1/tokens"),
        ("GET", "/v1/tokens"),
        ("DELETE", "/v1/tokens/1"),
        ("PUT", "/v1/permissions/global/0/shell"),
        ("POST", "/v1/accounts"),
    ] {
        let x = r.tcp(m, p, &web).await;
        assert_eq!(x.status, 403, "{m} {p}: {}", x.body);
        assert_eq!(code(&x), "insufficient_scope", "{m} {p}");
    }
    // After revocation the web token is rejected.
    let id = r
        .local("GET", "/v1/tokens", None)
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "w")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        r.local("DELETE", &format!("/v1/tokens/{id}"), None)
            .await
            .status,
        204
    );
    assert_eq!(r.tcp("POST", decision, &web).await.status, 401);
    let _ = (&r.env, http::<tokio::net::TcpStream>);
}

#[tokio::test]
async fn store_rules_for_web_kind() {
    use agentd_auth::AuthError;
    let e = env();
    let mk = |scope: Scope, ttl: Option<u32>| agentd_auth::NewToken {
        label: "w".into(),
        kind: TokenKind::Web,
        scope,
        ttl_days: ttl,
        allowlist: None,
        created_by: None,
    };
    // Default lifetime is 7 days, bounds 1 and 30 are allowed.
    let d = e.store.issue(mk(Scope::Decide, None)).await.unwrap();
    assert_eq!(d.expires_at, Some(T0 + 7 * 86_400));
    assert!(e.store.issue(mk(Scope::Chat, Some(1))).await.is_ok());
    let m = e.store.issue(mk(Scope::Decide, Some(30))).await.unwrap();
    assert_eq!(m.expires_at, Some(T0 + 30 * 86_400));
    // Out of the lifetime bounds.
    assert!(matches!(
        e.store.issue(mk(Scope::Decide, Some(31))).await,
        Err(AuthError::TtlTooLong)
    ));
    assert!(matches!(
        e.store.issue(mk(Scope::Decide, Some(0))).await,
        Err(AuthError::TtlTooLong)
    ));
    // A web token is only `chat` or `decide`.
    for scope in [Scope::Admin, Scope::Read] {
        assert!(
            matches!(
                e.store.issue(mk(scope, None)).await,
                Err(AuthError::ScopeNotAllowed)
            ),
            "{scope:?}"
        );
    }
    // The issued token is recognised as a web token with the decide right.
    let p = e.store.verify(d.value.expose()).await.unwrap();
    assert!(p.can_decide());
    assert_eq!((p.kind, p.scope), (TokenKind::Web, Scope::Decide));
}
