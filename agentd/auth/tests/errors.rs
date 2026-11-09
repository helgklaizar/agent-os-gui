//! `error_format_all_codes`: one request per code of api.md section 3; body `{error:{code,message,details,retryable,request_id}}`.

mod common;

use std::sync::Arc;

use agentd_api::{MigrationStatus, MigrationView, Server, ServerConfig, Services};
use agentd_core::{
    ExternalTokenRequest, IssuedExternal, PortFuture, RevokeOutcome, Scope, TokenAdmin,
    TokenAdminError, TokenInfo, TokenKind,
};
use common::{Env, Resp, env, http_body, new_token};
use serde_json::json;

/// Port implementation whose handler panics: checks `500 internal` without breaking the product.
struct Panicking;

impl TokenAdmin for Panicking {
    fn list(&self) -> PortFuture<'_, Result<Vec<TokenInfo>, TokenAdminError>> {
        Box::pin(async move { panic!("test handler panic") })
    }

    fn issue_external(
        &self,
        _r: ExternalTokenRequest,
    ) -> PortFuture<'_, Result<IssuedExternal, TokenAdminError>> {
        Box::pin(async move { Err(TokenAdminError::Unavailable) })
    }

    fn revoke(&self, _id: i64) -> PortFuture<'_, Result<RevokeOutcome, TokenAdminError>> {
        Box::pin(async move { Err(TokenAdminError::Unavailable) })
    }
}

struct Rig {
    env: Env,
    server: Server,
    tcp: std::net::SocketAddr,
    admin: String,
    read: String,
    module: String,
}

async fn tok(env: &Env, kind: TokenKind, scope: Scope, allow: Option<Vec<String>>) -> String {
    let mut n = new_token(kind, scope);
    n.allowlist = allow;
    env.store.issue(n).await.unwrap().value.expose().clone()
}

async fn rig() -> Rig {
    let env = env();
    let admin = tok(&env, TokenKind::Gui, Scope::Admin, None).await;
    let read = tok(&env, TokenKind::External, Scope::Read, None).await;
    let module = tok(&env, TokenKind::Module, Scope::Read, Some(vec![])).await;
    let mut cfg = ServerConfig::new(env.dir.path().join("agentd.sock"));
    cfg.tcp_addr = "127.0.0.1:0".parse().unwrap();
    cfg.tcp_enabled = true;
    cfg.clock = env.clock.clone();
    let server = Server::start_booting(cfg).await.unwrap();
    let mut s = Services::new(env.store.clone());
    s.tokens = Some(env.store.clone());
    s.status = Some(env.db.clone());
    s.settings = Some(Arc::new(agentd_db::SettingsRepo::new(
        env.db.clone(),
        env.clock.clone(),
    )));
    server.set_ready(s);
    let tcp = loop {
        if let Some(a) = server.tcp().local_addr() {
            break a;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    Rig {
        env,
        server,
        tcp,
        admin,
        read,
        module,
    }
}

async fn unix(
    sock: &std::path::Path,
    m: &str,
    p: &str,
    token: Option<&str>,
    extra: &[&str],
    body: &str,
) -> Resp {
    let mut s = tokio::net::UnixStream::connect(sock).await.unwrap();
    let mut hs: Vec<String> = token
        .map(|t| format!("Authorization: Bearer {t}"))
        .into_iter()
        .collect();
    hs.push(format!("Content-Length: {}", body.len()));
    hs.extend(extra.iter().map(|e| (*e).to_string()));
    http_body(&mut s, m, p, "localhost", &hs, body)
        .await
        .expect("response")
}

fn expect(r: &Resp, status: u16, code: &str, retryable: bool) {
    assert_eq!(r.status, status, "{code}: {}", r.body);
    let e = &r.json()["error"];
    assert_eq!(e["code"], code, "{}", r.body);
    assert!(
        e["message"].as_str().is_some_and(|m| !m.is_empty()),
        "message: {}",
        r.body
    );
    assert!(e["details"].is_object(), "details: {}", r.body);
    assert_eq!(e["retryable"], retryable, "{code} retryable");
    assert_eq!(
        e["request_id"].as_str(),
        r.header("x-request-id"),
        "{code}: request_id"
    );
    assert!(r.header("x-agentd-version").is_some());
    assert!(
        !r.body.contains("panicked") && !r.body.contains("src/"),
        "internals must not leak: {}",
        r.body
    );
}

#[tokio::test]
async fn error_format_all_codes() {
    let r = rig().await;
    let sock = r.server.socket_path().to_path_buf();
    let s = sock.as_path();
    let (a, rd, md) = (r.admin.as_str(), r.read.as_str(), r.module.as_str());
    expect(
        &unix(s, "POST", "/v1/tokens", Some(a), &[], "{oops").await,
        400,
        "bad_request",
        false,
    );
    expect(
        &unix(s, "GET", "/v1/tokens", None, &[], "").await,
        401,
        "unauthorized",
        false,
    );
    expect(
        &unix(
            s,
            "PATCH",
            "/v1/settings/about",
            Some(a),
            &["If-Match: 0"],
            "{}",
        )
        .await,
        403,
        "permission_denied",
        false,
    );
    expect(
        &unix(s, "POST", "/v1/tokens", Some(rd), &[], "{}").await,
        403,
        "insufficient_scope",
        false,
    );
    expect(
        &unix(s, "GET", "/v1/sessions", Some(md), &[], "").await,
        403,
        "forbidden_method",
        false,
    );
    let mut t = tokio::net::TcpStream::connect(r.tcp).await.unwrap();
    let origin = vec![
        "Origin: http://evil.example".to_string(),
        format!("Authorization: Bearer {a}"),
    ];
    let o = http_body(
        &mut t,
        "GET",
        "/v1/settings/theme",
        &r.tcp.to_string(),
        &origin,
        "",
    )
    .await
    .unwrap();
    expect(&o, 403, "forbidden_origin", false);
    expect(
        &unix(s, "GET", "/v1/nowhere", Some(a), &[], "").await,
        404,
        "not_found",
        false,
    );
    expect(
        &unix(s, "PUT", "/v1/sessions", Some(a), &[], "").await,
        405,
        "method_not_allowed",
        false,
    );
    let issued = unix(
        s,
        "POST",
        "/v1/tokens",
        Some(a),
        &[],
        &json!({"name": "x", "scope": "read"}).to_string(),
    )
    .await
    .json();
    let id = issued["id"].as_i64().unwrap();
    assert_eq!(
        unix(s, "DELETE", &format!("/v1/tokens/{id}"), Some(a), &[], "")
            .await
            .status,
        204
    );
    expect(
        &unix(s, "DELETE", &format!("/v1/tokens/{id}"), Some(a), &[], "").await,
        409,
        "conflict",
        false,
    );
    expect(
        &unix(
            s,
            "PATCH",
            "/v1/settings/theme",
            Some(a),
            &["If-Match: 9"],
            "{\"theme\":\"dark\"}",
        )
        .await,
        412,
        "precondition_failed",
        false,
    );
    let big = format!(
        "{{\"name\":\"{}\",\"scope\":\"read\"}}",
        "x".repeat(2 * 1024 * 1024)
    );
    expect(
        &unix(s, "POST", "/v1/tokens", Some(a), &[], &big).await,
        413,
        "payload_too_large",
        false,
    );
    expect(
        &unix(
            s,
            "POST",
            "/v1/tokens",
            Some(a),
            &[],
            "{\"name\":\"x\",\"scope\":\"admin\"}",
        )
        .await,
        422,
        "validation_failed",
        false,
    );
    expect(
        &unix(
            s,
            "PATCH",
            "/v1/settings/theme",
            Some(a),
            &[],
            "{\"theme\":\"dark\"}",
        )
        .await,
        428,
        "precondition_required",
        false,
    );
    // 429: six failed tokens over TCP from one address.
    let mut last = None;
    for i in 0..7 {
        let mut c = tokio::net::TcpStream::connect(r.tcp).await.unwrap();
        let bad = vec![format!("Authorization: Bearer agt_{i:0>43}")];
        last = http_body(
            &mut c,
            "GET",
            "/v1/settings/theme",
            &r.tcp.to_string(),
            &bad,
            "",
        )
        .await;
    }
    let limited = last.unwrap();
    expect(&limited, 429, "rate_limited", true);
    assert!(limited.header("retry-after").is_some());

    // 500: handler panic; 503 core_down: token service not connected; 503 migrating: migration in progress.
    let dir2 = tempfile::tempdir().unwrap();
    let mut cfg = ServerConfig::new(dir2.path().join("b.sock"));
    cfg.clock = r.env.clock.clone();
    let s2 = Server::start_booting(cfg).await.unwrap();
    let mut svc = Services::new(r.env.store.clone());
    svc.tokens = Some(Arc::new(Panicking));
    s2.set_ready(svc);
    let sock2 = s2.socket_path().to_path_buf();
    expect(
        &unix(&sock2, "GET", "/v1/tokens", Some(a), &[], "").await,
        500,
        "internal",
        true,
    );
    expect(
        &unix(&sock2, "GET", "/v1/settings/theme", Some(a), &[], "").await,
        503,
        "core_down",
        true,
    );
    s2.shutdown().await;
    let dir3 = tempfile::tempdir().unwrap();
    let s3 = Server::start_booting(ServerConfig::new(dir3.path().join("c.sock")))
        .await
        .unwrap();
    s3.health().set_migration(Some(MigrationView {
        status: MigrationStatus::Running,
        from: 0,
        to: 1,
        step: None,
        progress: Some(0.1),
    }));
    let m = unix(s3.socket_path(), "GET", "/v1/tokens", Some(a), &[], "").await;
    expect(&m, 503, "migrating", true);
    assert!(m.header("retry-after").is_some());
    s3.shutdown().await;
    r.server.shutdown().await;
}
