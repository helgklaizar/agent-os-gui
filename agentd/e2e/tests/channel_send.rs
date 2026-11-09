//! A5-06: outgoing `send` through a channel: permissions, delivery outcomes, unknown outcome, receiving incoming messages by polling.

mod common;

use std::time::Duration;

use common::{Env, finished};
use serde_json::{Value, json};

const KIND: &str = "scripted-channel";

async fn chan(e: &Env, body: Value) -> Value {
    let (st, v) = e.admin.post("/_test/channel", body).await;
    assert_eq!(st, 200, "{v}");
    v
}

async fn account(e: &Env) -> i64 {
    e.admin
        .make("/accounts", json!({"type": KIND, "label": "channel"}))
        .await["id"]
        .as_i64()
        .unwrap()
}

async fn allow_send(e: &Env) {
    let (st, v) = e
        .admin
        .put("/permissions/global/0/send", json!({"mode": "allow"}))
        .await;
    assert_eq!(st, 200, "{v}");
}

async fn connected(e: &Env) -> i64 {
    chan(e, json!({"mode": "qr"})).await;
    let a = account(e).await;
    e.admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    chan(e, json!({"scanned": true, "refresh": a})).await;
    a
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_send_delivered_after_right() {
    let mut e = Env::start().await;
    connected(&e).await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send_chan write").await;
    let (id, _) = e.wait_card(s).await;
    assert_eq!(
        chan(&e, json!({})).await["sends"],
        0,
        "does not go out without a decision"
    );
    assert_eq!(e.decide(id, json!({"decision": "allow"})).await.0, 200);
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
    assert_eq!(chan(&e, json!({})).await["sends"], 1);
    let rows: Vec<(String, Option<String>, String)> = e
        .inspect(|c| {
            c.prepare("SELECT direction, routed_session_id, body FROM gw_messages")?
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, Option<String>>(1).ok().flatten(),
                        r.get(2)?,
                    ))
                })?
                .collect()
        })
        .await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "out");
    let rev: Vec<i64> = e
        .inspect(|c| {
            c.prepare("SELECT reversible FROM actions_log WHERE action = 'send'")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(rev, [0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_unknown_outcome_not_retried() {
    let e = Env::start().await;
    connected(&e).await;
    allow_send(&e).await;
    chan(&e, json!({"outcomes": ["unknown", "delivered"]})).await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send_chan write").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "delivery_unknown");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(chan(&e, json!({})).await["sends"], 1, "no auto-retry");
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_rejected_is_reported_and_not_stored_as_sent() {
    let mut e = Env::start().await;
    connected(&e).await;
    allow_send(&e).await;
    chan(&e, json!({"outcomes": ["rejected"]})).await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send_chan write").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "rejected");
    let out: i64 = e
        .inspect(|c| {
            c.query_row(
                "SELECT count(*) FROM gw_messages WHERE direction = 'out'",
                [],
                |r| r.get(0),
            )
        })
        .await;
    assert_eq!(out, 0, "a rejected message is not recorded as sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_send_without_connected_account_fails_closed() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "qr"})).await;
    account(&e).await;
    allow_send(&e).await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send_chan write").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "no_connector");
    assert_eq!(chan(&e, json!({})).await["sends"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_incoming_poll_goes_through_gateway() {
    let e = Env::start().await;
    let a = connected(&e).await;
    chan(
        &e,
        json!({"incoming": [{"ext_id": "u1", "handle": "@bob", "body": "hello from the channel"},
                            {"ext_id": "u1", "handle": "@bob", "body": "repeat"}]}),
    )
    .await;
    chan(&e, json!({"poll": true})).await;
    let (_, v) = e.read.get(&format!("/events?kind=msg&sub={KIND}")).await;
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        1,
        "repeated ext_id dropped: {v}"
    );
    assert!(
        v["items"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello from the channel")
    );
    assert!(a > 0);
}
