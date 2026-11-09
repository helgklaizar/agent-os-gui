//! Core client against `fake-agentd`: subscriptions, drop and reconnect, outbound queue.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{client, next_frame, quiet, start, wait_event, wait_state};
use core_client::api::{BarSys, ErrorCode, FrameKind};
use core_client::{ConnState, CoreEvent};
use fake_agentd::{Config, FrameType, Scenario};
use serde_json::json;

fn scenario(name: &str) -> Scenario {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fake-agentd/scenarios")
        .join(name);
    match Scenario::from_file(&path) {
        Ok(s) => s,
        Err(e) => panic!("scenario {name}: {e}"),
    }
}

#[tokio::test]
async fn client_subscribes_and_receives_bar_sys() {
    let d = start(Config::temp().bar_sys_period(Duration::from_millis(40))).await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.sys").is_ok());
    let snap = next_frame(&rx, "bar.sys").await;
    let Ok(snap) = snap else { panic!("{snap:?}") };
    assert_eq!(snap.kind, FrameKind::Snapshot, "snapshot first");
    assert!(
        serde_json::from_value::<BarSys>(snap.data.clone())
            .is_ok_and(|b| b.cpu.is_some() && b.temp.is_some()),
        "bar.sys fields: {:?}",
        snap.data
    );
    let mut last = snap.seq;
    let mut values = vec![snap.data.clone()];
    for _ in 0..3 {
        let f = next_frame(&rx, "bar.sys").await;
        let Ok(f) = f else { panic!("{f:?}") };
        assert_eq!(f.kind, FrameKind::Delta, "deltas after that");
        assert_eq!(f.seq, last + 1, "seq grows by 1");
        last = f.seq;
        values.push(f.data);
    }
    assert!(values.windows(2).any(|w| w[0] != w[1]), "values change");
    // unsubscribing stops frames
    assert!(c.unsubscribe("bar.sys").is_ok());
    tokio::time::sleep(Duration::from_millis(150)).await;
    while rx.try_recv().is_ok() {}
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rx.try_recv().is_err(), "no frames after unsubscribe");
    c.shutdown();
}

#[tokio::test]
async fn client_survives_ws_drop() {
    let d = start(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .scenario(scenario("bar-then-drop.toml")),
    )
    .await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.sys").is_ok());
    let first = next_frame(&rx, "bar.sys").await;
    assert!(
        matches!(&first, Ok(f) if f.kind == FrameKind::Snapshot && f.data["cpu"] == json!(14)),
        "snapshot with the scenario value: {first:?}"
    );

    d.advance(Duration::from_millis(5000));
    assert!(
        wait_state(&rx, ConnState::Reconnecting).await.is_ok(),
        "a drop gives Reconnecting"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(d.ws_connections(), 0, "no connections during the drop");

    d.advance(Duration::from_millis(2000));
    assert!(
        wait_state(&rx, ConnState::Connected).await.is_ok(),
        "after restore Connected again"
    );
    let again = next_frame(&rx, "bar.sys").await;
    assert!(
        matches!(&again, Ok(f) if f.kind == FrameKind::Snapshot && f.seq == 1),
        "after the return a snapshot again, the subscription is restored: {again:?}"
    );
    d.emit(
        "bar.sys",
        FrameType::Delta,
        json!({ "cpu": 77, "gpu": 1, "ram": 2, "disk": 3, "temp": 4 }),
    );
    let upd = next_frame(&rx, "bar.sys").await;
    assert!(
        matches!(&upd, Ok(f) if f.kind == FrameKind::Delta && f.data["cpu"] == json!(77)),
        "values updated: {upd:?}"
    );
    c.shutdown();
}

#[tokio::test]
async fn client_queue_no_duplicates() {
    let d = quiet().await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    d.drop_ws();
    assert!(wait_state(&rx, ConnState::Reconnecting).await.is_ok());
    let ids: Vec<String> = ["one", "two", "three"]
        .iter()
        .filter_map(|t| c.enqueue_echo(t).ok())
        .collect();
    assert_eq!(ids.len(), 3, "three items queued");
    assert!(
        ids.iter()
            .all(|i| uuid::Uuid::parse_str(i).is_ok_and(|u| u.get_version_num() == 7)),
        "client_id — UUID v7: {ids:?}"
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(d.echo_log().is_empty(), "nothing went out during the drop");
    d.restore();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    for id in &ids {
        let got = wait_event(
            &rx,
            "delivery",
            |e| matches!(e, CoreEvent::Delivered { client_id, .. } if client_id == id),
        )
        .await;
        assert!(
            matches!(
                &got,
                Ok(CoreEvent::Delivered {
                    duplicate: false,
                    ..
                })
            ),
            "{got:?}"
        );
    }
    let log = d.echo_log();
    assert_eq!(
        log.iter().map(|e| e.client_id.clone()).collect::<Vec<_>>(),
        ids,
        "exactly three, order preserved"
    );
    assert_eq!(
        log.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(d.echo_log().len(), 3, "no repeats");
    c.shutdown();
}

#[tokio::test]
async fn client_queue_same_client_id_once() {
    let d = quiet().await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    // both echoes are queued during the gap: the duplicate must be dropped by the client queue, not by the mock
    d.drop_ws();
    assert!(wait_state(&rx, ConnState::Reconnecting).await.is_ok());
    assert!(c.enqueue_echo_with_id("same-id", "hello").is_ok());
    assert!(c.enqueue_echo_with_id("same-id", "hello").is_ok());
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(d.echo_log().is_empty(), "nothing went out during the drop");
    d.restore();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    let first = wait_event(&rx, "first delivery", |e| {
        matches!(e, CoreEvent::Delivered { .. })
    })
    .await;
    assert!(
        matches!(&first, Ok(CoreEvent::Delivered { client_id, duplicate: false }) if client_id == "same-id"),
        "{first:?}"
    );
    // the client sent exactly one request: no second Delivered (including duplicate: true), no second mock record
    let mut extra = Vec::new();
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(400), rx.recv()).await {
        if matches!(ev, CoreEvent::Delivered { .. } | CoreEvent::Failed { .. }) {
            extra.push(format!("{ev:?}"));
        }
    }
    assert!(
        extra.is_empty(),
        "the client queue sent a duplicate: {extra:?}"
    );
    assert_eq!(d.echo_log().len(), 1, "one result");
    // the mock returns a direct repeat without a duplicate
    let http = common::http(&d, Some(d.admin_token()));
    let rep = http.echo("same-id", "hello").await;
    assert!(
        matches!(&rep, Ok(r) if r.duplicate && r.ordinal == 1),
        "repeat of the same client_id and text: {rep:?}"
    );
    assert_eq!(d.echo_log().len(), 1, "after a direct repeat still one");
    // the same client_id with different text: 409 client_id_conflict
    assert!(c.enqueue_echo_with_id("same-id", "other text").is_ok());
    let failed = wait_event(&rx, "conflict", |e| matches!(e, CoreEvent::Failed { .. })).await;
    assert!(
        matches!(&failed, Ok(CoreEvent::Failed { error, .. }) if error.status == 409 && error.code == ErrorCode::ClientIdConflict),
        "{failed:?}"
    );
    assert_eq!(d.echo_log().len(), 1);
    c.shutdown();
}

#[tokio::test]
async fn client_4xx_drops_item_and_continues() {
    let d = quiet().await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(
        c.enqueue_echo_with_id("bad", "").is_ok(),
        "an empty text is rejected by the mock with 422"
    );
    assert!(c.enqueue_echo_with_id("good", "ok").is_ok());
    let failed = wait_event(
        &rx,
        "422",
        |e| matches!(e, CoreEvent::Failed { client_id, .. } if client_id == "bad"),
    )
    .await;
    assert!(
        matches!(&failed, Ok(CoreEvent::Failed { error, .. }) if error.status == 422),
        "{failed:?}"
    );
    let ok = wait_event(
        &rx,
        "next",
        |e| matches!(e, CoreEvent::Delivered { client_id, .. } if client_id == "good"),
    )
    .await;
    assert!(ok.is_ok(), "4xx does not block the queue: {ok:?}");
    c.shutdown();
}
