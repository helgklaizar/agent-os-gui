//! A7-10: `POST /logout` (admin, local only): a log entry, closing all WS with code `1001`, ending the user session
//! with `loginctl` (in the test build `tunnel-fake` stands in for it; its invocation is visible in the stub log).

mod common;

use std::time::Duration;

use common::Env;
use core_client::ConnState;
use core_client::CoreEvent;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn logout_closes_websockets_logs_and_runs_session_command() {
    let mut e = Env::start().await;
    let ws = e.ws();
    let rx = ws.client.events();
    let (st, v) = e.admin.post("/logout", json!({})).await;
    assert_eq!(st, 202, "{v}");
    let end = std::time::Instant::now() + Duration::from_secs(8);
    let mut closed = false;
    while std::time::Instant::now() < end && !closed {
        match rx.try_recv() {
            Ok(CoreEvent::State(
                ConnState::Reconnecting | ConnState::Connecting | ConnState::Offline,
            )) => closed = true,
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    assert!(closed, "WS closed by the core");
    let mut log = String::new();
    for _ in 0..60 {
        log = std::fs::read_to_string(e.home.join("tunnel.log")).unwrap_or_default();
        if log.contains("terminate-session") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        log.lines()
            .any(|l| l.starts_with("start terminate-session c1 ")),
        "session-end command: {log:?}"
    );
    let rows: Vec<(String, String)> = e
        .inspect(|c| {
            c.prepare("SELECT action, actor FROM actions_log WHERE action = 'logout'")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .await;
    assert_eq!(rows, [("logout".to_string(), "user".to_string())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_is_admin_only() {
    let e = Env::start().await;
    for api in [&e.read, &e.chat] {
        assert_eq!(api.post("/logout", json!({})).await.0, 403);
    }
}
