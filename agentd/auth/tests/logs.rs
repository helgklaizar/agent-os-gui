//! `logs_contain_no_secrets`: with `RUST_LOG=trace` (TRACE level) the core log has no token values, their hashes,
//! the `Authorization` header, fixture secrets or harmful strings from requests.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use agentd_api::{Server, ServerConfig, Services};
use agentd_core::{Scope, TokenKind};
use common::{env, http_body, new_token};
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct W(Arc<Mutex<Vec<u8>>>);

impl Write for W {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const EVIL: &[&str] = &[
    "'; DROP TABLE api_token;--",
    "<script>alert(1)</script>",
    "../../etc/passwd",
    "\u{202e}rtl-override",
    "FIXTURE-API-KEY-0123456789-not-real",
    "HARMFUL-STRING-NAME",
    "${jndi:ldap://x}",
];

fn hex(t: &str) -> String {
    Sha256::digest(t.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_contain_no_secrets() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || W(sink.clone()))
        .finish();
    tracing::subscriber::set_global_default(sub).unwrap();

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
    let sock = server.socket_path().to_path_buf();
    let call =
        |m: &'static str, p: String, auth: Option<String>, extra: Vec<String>, body: String| {
            let sock = sock.clone();
            async move {
                let mut c = tokio::net::UnixStream::connect(sock).await.unwrap();
                let mut hs: Vec<String> = auth
                    .into_iter()
                    .map(|t| format!("Authorization: Bearer {t}"))
                    .collect();
                hs.push(format!("Content-Length: {}", body.len()));
                hs.extend(extra);
                http_body(&mut c, m, &p, "localhost", &hs, &body).await
            }
        };

    // Issue a token with a harmful name, use it, revoke it.
    let issued = call(
        "POST",
        "/v1/tokens".into(),
        Some(admin.clone()),
        vec![],
        json!({"name": EVIL[5], "scope": "read"}).to_string(),
    )
    .await
    .unwrap()
    .json();
    let (id, value) = (
        issued["id"].as_i64().unwrap(),
        issued["token"].as_str().unwrap().to_string(),
    );
    call(
        "GET",
        "/v1/settings/theme".into(),
        Some(value.clone()),
        vec![],
        String::new(),
    )
    .await;
    // Token in the URL, bad token, harmful header and setting values.
    let in_url = format!("agt_{}", "Q".repeat(43));
    call(
        "GET",
        format!("/v1/settings/theme?token={in_url}"),
        None,
        vec![],
        String::new(),
    )
    .await;
    call(
        "GET",
        format!("/v1/sessions?access_token={}", EVIL[4]),
        None,
        vec![],
        String::new(),
    )
    .await;
    let bad = format!("agt_{}", "Z".repeat(43));
    call(
        "GET",
        "/v1/tokens".into(),
        Some(bad.clone()),
        vec![format!("X-Request-Id: {}", EVIL[0])],
        String::new(),
    )
    .await;
    for (i, e) in EVIL.iter().enumerate() {
        call(
            "PATCH",
            "/v1/settings/models".into(),
            Some(admin.clone()),
            vec![format!("If-Match: {i}")],
            json!({ "k": e }).to_string(),
        )
        .await;
        call(
            "GET",
            format!("/v1/{e}"),
            Some(admin.clone()),
            vec![],
            String::new(),
        )
        .await;
    }
    let stream = tokio::net::UnixStream::connect(server.socket_path())
        .await
        .unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {value}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    call(
        "DELETE",
        format!("/v1/tokens/{id}"),
        Some(admin.clone()),
        vec![],
        String::new(),
    )
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await;
    server.shutdown().await;

    let log = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    assert!(
        log.lines().count() > 10,
        "log is not empty ({} lines): the check is meaningful",
        log.lines().count()
    );
    assert!(
        log.contains("token_issued") && log.contains("token_revoked"),
        "issue and revoke events are recorded"
    );
    let mut secrets: Vec<String> = vec![admin.clone(), value.clone(), bad, in_url];
    secrets.extend(secrets.clone().iter().map(|t| hex(t)));
    secrets.extend(EVIL.iter().map(|e| (*e).to_string()));
    secrets.push("Authorization: Bearer".into());
    secrets.push("agt_".into());
    for s in &secrets {
        assert!(!log.contains(s.as_str()), "found \"{s}\" in the log");
    }
}
