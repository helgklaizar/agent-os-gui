//! A1-11: a build with `test-support`: clock advance and the `test` token kind.
#![cfg(feature = "test-support")]

mod common;

use std::sync::Arc;

use agentd_core::{Clock, ManualClock};
use common::{TOKEN_CHAT, TOKEN_CHAT_ONLY, TOKEN_READ, config, start, unix_body, unix_req};

#[tokio::test]
async fn advance_moves_manual_clock() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(1_800_000_000));
    let mut cfg = config(dir.path());
    cfg.clock = clock.clone();
    let s = start(cfg).await;
    let p = s.socket_path();
    let r = unix_body(
        p,
        "POST",
        "/v1/_test/advance",
        TOKEN_CHAT,
        &[],
        br#"{"seconds": 90}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(clock.now_unix(), 1_800_000_090);
    assert_eq!(r.json()["now"], 1_800_000_090);
    for tok in [TOKEN_READ, TOKEN_CHAT_ONLY] {
        let r = unix_body(
            p,
            "POST",
            "/v1/_test/advance",
            tok,
            &[],
            br#"{"seconds": 1}"#,
        )
        .await;
        assert_eq!(r.status, 403);
    }
    let bad = unix_body(
        p,
        "POST",
        "/v1/_test/advance",
        TOKEN_CHAT,
        &[],
        br#"{"seconds": -5}"#,
    )
    .await;
    assert_eq!(bad.status, 422);
    assert_eq!(
        unix_req(p, "POST", "/v1/_test/advance", None).await.status,
        401
    );
    s.shutdown().await;
}

#[tokio::test]
async fn advance_on_system_clock_is_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(config(dir.path())).await;
    let r = unix_body(
        s.socket_path(),
        "POST",
        "/v1/_test/advance",
        TOKEN_CHAT,
        &[],
        br#"{"seconds": 1}"#,
    )
    .await;
    assert_eq!(r.status, 409);
    s.shutdown().await;
}
