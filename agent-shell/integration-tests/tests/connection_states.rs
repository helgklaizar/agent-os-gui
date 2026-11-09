//! Client connection states against `fake-agentd`: `core_down`, migration, token refusal, API version, pong, `seq` gap, TCP.

mod common;

use std::time::Duration;

use common::{
    client, core_config, core_config_with, frame_of, next_frame, quiet, start, wait_event,
    wait_state,
};
use core_client::api::{FrameKind, Theme};
use core_client::{ConnState, CoreClient, CoreEvent, OfflineReason, Token, TokenSource};
use fake_agentd::{Config, CoreMode, FrameType};
use serde_json::json;

#[tokio::test]
async fn client_core_down_state_then_recovers() {
    let d = quiet().await;
    d.set_mode(CoreMode::Down);
    let c = client(&d);
    let rx = c.events();
    assert!(
        wait_state(&rx, ConnState::CoreUnavailable).await.is_ok(),
        "core_down gives a separate state"
    );
    let attempts = d.requests().len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        d.requests().len() > attempts,
        "retries continue without a limit"
    );
    d.set_mode(CoreMode::Ok);
    assert!(
        wait_state(&rx, ConnState::Connected).await.is_ok(),
        "the core came up: Connected"
    );
    c.shutdown();
}

#[tokio::test]
async fn client_migrating_is_not_offline() {
    let d = quiet().await;
    d.set_mode(CoreMode::Migrating);
    let c = client(&d);
    let rx = c.events();
    let m = wait_event(&rx, "migrating", |e| matches!(e, CoreEvent::Migrating(_))).await;
    assert!(
        matches!(&m, Ok(CoreEvent::Migrating(Some(mig))) if mig.status == "running" && mig.progress.is_some()),
        "{m:?}"
    );
    d.set_mode(CoreMode::Ok);
    let seen = wait_event(&rx, "Connected without Offline", |e| {
        matches!(
            e,
            CoreEvent::State(ConnState::Connected | ConnState::Offline)
        )
    })
    .await;
    assert!(
        matches!(seen, Ok(CoreEvent::State(ConnState::Connected))),
        "Offline is not shown during a migration"
    );
    c.shutdown();
}

#[tokio::test]
async fn client_unknown_token_goes_offline_and_stops() {
    let d = quiet().await;
    let c = match CoreClient::start(core_config(&d, "agt_unknown-token-value")) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let rx = c.events();
    let r = wait_event(&rx, "reason", |e| matches!(e, CoreEvent::OfflineReason(_))).await;
    assert_eq!(
        r.ok(),
        Some(CoreEvent::OfflineReason(OfflineReason::TokenRejected))
    );
    assert!(wait_state(&rx, ConnState::Offline).await.is_ok());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let n = d.requests().len();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(d.requests().len(), n, "no reconnect after 401");
    c.shutdown();
}

#[tokio::test]
async fn client_revoked_token_closed_4401_goes_offline() {
    let d = quiet().await;
    let issued = d.issue(fake_agentd::TokenSpec::new(
        fake_agentd::Scope::Read,
        fake_agentd::Kind::Test,
    ));
    let Ok(issued) = issued else {
        panic!("{issued:?}")
    };
    let c = match CoreClient::start(core_config(&d, &issued.token)) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    d.revoke(issued.id);
    let r = wait_event(&rx, "reason", |e| matches!(e, CoreEvent::OfflineReason(_))).await;
    assert_eq!(
        r.ok(),
        Some(CoreEvent::OfflineReason(OfflineReason::TokenRejected)),
        "4401: do not retry"
    );
    assert!(wait_state(&rx, ConnState::Offline).await.is_ok());
    c.shutdown();
}

#[tokio::test]
async fn client_unsupported_api_major_goes_offline() {
    let d = start(Config::temp().no_ticker().api_major(2)).await;
    let c = client(&d);
    let rx = c.events();
    let r = wait_event(&rx, "reason", |e| matches!(e, CoreEvent::OfflineReason(_))).await;
    assert_eq!(
        r.ok(),
        Some(CoreEvent::OfflineReason(
            OfflineReason::ApiVersionUnsupported
        ))
    );
    assert!(c.subscribe("bar.sys").is_ok());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        d.ws_connections(),
        0,
        "no subscribing to an incompatible core"
    );
    c.shutdown();
}

#[tokio::test]
async fn client_pong_timeout_reconnects() {
    let d = quiet().await;
    let mut cfg = core_config(&d, d.admin_token());
    cfg.ping_interval = Duration::from_millis(40);
    let c = match CoreClient::start(cfg) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        d.ws_connections(),
        1,
        "while pongs arrive the connection lives"
    );
    d.set_pongs(false);
    assert!(
        wait_state(&rx, ConnState::Reconnecting).await.is_ok(),
        "two missed pongs mean a drop"
    );
    d.set_pongs(true);
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    c.shutdown();
}

#[tokio::test]
async fn client_seq_gap_resubscribes() {
    let d = quiet().await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("events").is_ok());
    let snap = next_frame(&rx, "events").await;
    assert!(
        matches!(&snap, Ok(f) if f.kind == FrameKind::Snapshot),
        "{snap:?}"
    );
    d.inject_seq_gap("events");
    d.emit(
        "events",
        FrameType::Event,
        json!({ "id": 1, "text": "lost" }),
    );
    let resync = wait_event(&rx, "new snapshot after the gap", |e| {
        frame_of(e).is_some_and(|f| f.topic == "events")
    })
    .await;
    let Ok(CoreEvent::Frame(f)) = resync else {
        panic!("{resync:?}")
    };
    assert_eq!(
        f.kind,
        FrameKind::Snapshot,
        "a frame with a seq gap is not delivered; a resubscribe and a snapshot follow instead"
    );
    d.emit("events", FrameType::Event, json!({ "id": 2 }));
    let next = next_frame(&rx, "events").await;
    assert!(
        matches!(&next, Ok(f) if f.kind == FrameKind::Event && f.seq == 2 && f.data["id"] == json!(2)),
        "{next:?}"
    );
    c.shutdown();
}

#[tokio::test]
async fn client_theme_roundtrip_over_tcp() {
    let d = start(Config::temp().no_ticker().with_tcp()).await;
    let tcp = common::tcp(&d);
    let http = core_client::HttpClient::new(tcp.clone(), Some(Token::new(d.admin_token())));
    let page = http.theme().await;
    assert!(matches!(&page, Ok(p) if p.rev == 1), "{page:?}");
    assert!(matches!(http.set_theme(1, Theme::Dark).await, Ok(p) if p.theme == Theme::Dark));
    // WS client over TCP
    let mut cfg = core_config_with(&d, TokenSource::Value(Token::new(d.admin_token())));
    cfg.endpoint = tcp;
    let c = match CoreClient::start(cfg) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    assert!(
        wait_state(&c.events(), ConnState::Connected).await.is_ok(),
        "the client connects over TCP too"
    );
    c.shutdown();
}
