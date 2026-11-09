//! A1-12: `e2e_core_client_roundtrip`: `core-client` from agent-shell against a running `agentd` binary over the socket:
//! request without a token, a `read` token on `PATCH`, WS with a snapshot and a delta, core stop, reconnect.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentd_auth::{NewToken, TokenPolicy, TokenStore};
use agentd_core::{Scope as CoreScope, SystemClock, TokenKind};
use agentd_db::{Db, DbConfig};
use core_client::api::{Scope, Theme, TokenCreate};
use core_client::client::HttpClient;
use core_client::http::Request;
#[path = "common/bin.rs"]
mod bin;

use bin::agentd_bin;
use core_client::{
    ClientError, ConnState, CoreClient, CoreConfig, CoreEvent, Endpoint, Token, TokenSource,
};

struct Daemon(Option<Child>);

impl Daemon {
    fn start(db: &Path, sock: &Path) -> Daemon {
        let daemon = Daemon(Some(
            Command::new(agentd_bin())
                .args(["serve", "--db"])
                .arg(db)
                .arg("--socket")
                .arg(sock)
                .env_remove("XDG_RUNTIME_DIR")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        ));
        let end = Instant::now() + Duration::from_secs(10);
        // Readiness is a `/health` reply with status ok (the socket may open already while migrating).
        while Instant::now() < end {
            if let Ok(mut s) = std::os::unix::net::UnixStream::connect(sock) {
                use std::io::{Read, Write};
                s.write_all(
                    b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .ok();
                let mut out = String::new();
                s.read_to_string(&mut out).ok();
                if out.contains("\"status\":\"ok\"") {
                    return daemon;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("agentd was not ready within 10 s");
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let st = Command::new("kill")
                .args(["-TERM", &c.id().to_string()])
                .status()
                .unwrap();
            assert!(st.success());
            assert!(c.wait().unwrap().success(), "clean shutdown");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            drop(c.kill());
            drop(c.wait());
        }
    }
}

/// Tokens are prepared before the core starts on the same DB (afterwards the DB file belongs to the core).
async fn prepare_tokens(db: &Path) -> (String, String) {
    let db = Arc::new(Db::open(DbConfig::new(db)).unwrap());
    let store = TokenStore::new(db, Arc::new(SystemClock), TokenPolicy::default());
    let mk = |kind, scope| NewToken {
        label: "e2e".into(),
        kind,
        scope,
        ttl_days: None,
        allowlist: None,
        created_by: None,
    };
    {
        let admin = store
            .issue(mk(TokenKind::Gui, CoreScope::Admin))
            .await
            .unwrap();
        let read = store
            .issue(mk(TokenKind::External, CoreScope::Read))
            .await
            .unwrap();
        (admin.value.expose().clone(), read.value.expose().clone())
    }
}

fn wait_event(c: &CoreClient, what: &str, mut pred: impl FnMut(&CoreEvent) -> bool) {
    let rx = c.events();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        match rx.try_recv() {
            Ok(ev) if pred(&ev) => return,
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    panic!("event not received: {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_core_client_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sock) = (
        dir.path().join("agentos/sessions.db"),
        dir.path().join("agentd.sock"),
    );
    let (admin, read) = prepare_tokens(&db).await;
    let mut daemon = Daemon::start(&db, &sock);
    let ep = Endpoint::Unix(sock.clone());

    // Without a token: health is anonymous, everything else is 401.
    let anon = HttpClient::new(ep.clone(), None);
    let h = anon.health().await.expect("health parsed by the client");
    assert_eq!(h.api.major, 1);
    match anon.call(Request::get("/v1/settings/theme")).await {
        Err(ClientError::Api(e)) => assert_eq!(e.status, 401),
        other => panic!("expected 401: {other:?}"),
    }
    // A read token reads but does not write.
    let reader = HttpClient::new(ep.clone(), Some(Token::new(read)));
    let page = reader.theme().await.unwrap();
    assert_eq!((page.rev, page.theme), (0, Theme::Auto));
    match reader.set_theme(page.rev, Theme::Dark).await {
        Err(ClientError::Api(e)) => {
            assert_eq!((e.status, e.code.as_str()), (403, "insufficient_scope"))
        }
        other => panic!("expected 403: {other:?}"),
    }
    // admin writes; a stale rev gives 412.
    let writer = HttpClient::new(ep.clone(), Some(Token::new(admin.clone())));
    let saved = writer.set_theme(0, Theme::Dark).await.unwrap();
    assert_eq!((saved.rev, saved.theme), (1, Theme::Dark));
    match writer.set_theme(0, Theme::Light).await {
        Err(ClientError::Api(e)) => assert_eq!(e.status, 412),
        other => panic!("expected 412: {other:?}"),
    }
    // Tokens: issue and list are parsed by the client types.
    let issued = writer
        .issue_token(&TokenCreate {
            name: "phone".into(),
            scope: Scope::Chat,
            ttl_days: Some(7),
            kind: None,
            allowlist: None,
        })
        .await
        .unwrap();
    assert!(issued.token.starts_with("agt_"));
    let list = writer.list_tokens().await.unwrap();
    assert!(list.iter().any(|t| t.id == issued.info.id));

    // WS: subscription, snapshot, delta.
    let c = CoreClient::start(CoreConfig::new(
        ep.clone(),
        TokenSource::Value(Token::new(admin)),
    ))
    .unwrap();
    wait_event(&c, "Connected", |e| {
        matches!(e, CoreEvent::State(ConnState::Connected))
    });
    c.subscribe("settings").unwrap();
    wait_event(
        &c,
        "snapshot settings",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "settings" && f.data["pages"]["theme"] == 1),
    );
    writer.set_theme(1, Theme::Light).await.unwrap();
    wait_event(
        &c,
        "delta settings",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "settings" && f.data["page"] == "theme" && f.data["rev"] == 2),
    );

    // Disconnect: the core is stopped; the client reconnects and re-subscribes by itself, the snapshot carries the saved state.
    daemon.stop();
    wait_event(&c, "Reconnecting", |e| {
        matches!(
            e,
            CoreEvent::State(ConnState::Reconnecting | ConnState::CoreUnavailable)
        )
    });
    let mut daemon = Daemon::start(&db, &sock);
    wait_event(&c, "Connected after restart", |e| {
        matches!(e, CoreEvent::State(ConnState::Connected))
    });
    wait_event(
        &c,
        "snapshot after restart",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "settings" && f.data["pages"]["theme"] == 2),
    );
    daemon.stop();
}
