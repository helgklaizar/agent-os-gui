//! S8 scenarios: the value and tooltip text come from `fake-agentd` over WS to `parse_frame`.

mod common;

use common::{client, http, send, wait_event};
use controllers::controllers::indicator::parse_frame;
use controllers::indicators::State;
use core_client::http::Request;
use core_client::{CoreEvent, HttpClient};
use serde_json::{Value, json};

async fn post(admin: &HttpClient, body: Value) -> u16 {
    send(admin, Request::post("/v1/_test/indicator-frame", body))
        .await
        .status
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_value_and_text_reach_subscriber_delta_and_late_snapshot() {
    let d = common::quiet().await;
    let admin = http(&d, Some(d.admin_token()));
    let core = client(&d);
    assert!(core.subscribe("bar.indicators").is_ok());
    let rx = core.events();
    wait_event(
        &rx,
        "snapshot",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "bar.indicators"),
    )
    .await
    .expect("snapshot");
    let body = json!({ "id": "disk", "state": "warn", "value": 82.5, "text": "/ used 82.5%" });
    assert_eq!(post(&admin, body).await, 200);
    let ev = wait_event(
        &rx,
        "delta",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "bar.indicators" && f.data["id"] == "disk"),
    )
    .await
    .expect("delta");
    let CoreEvent::Frame(f) = ev else {
        panic!("not a frame")
    };
    let parsed = parse_frame(&f.data).expect("frame parsed");
    assert_eq!(parsed.state, State::Warn);
    assert_eq!(parsed.value, Some(82.5));
    assert_eq!(parsed.text.as_deref(), Some("/ used 82.5%"));
    assert!(f.data["ts"].is_i64(), "ts: {}", f.data);

    let late = client(&d);
    assert!(late.subscribe("bar.indicators").is_ok());
    let rx2 = late.events();
    let snap = wait_event(
        &rx2,
        "late snapshot",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "bar.indicators"),
    )
    .await
    .expect("snapshot");
    let CoreEvent::Frame(s) = snap else {
        panic!("not a frame")
    };
    let item = s.data["items"]
        .as_array()
        .and_then(|a| a.iter().find(|i| i["id"] == "disk").cloned())
        .expect("disk in the snapshot");
    assert_eq!(item["value"], json!(82.5));
    assert_eq!(item["text"], json!("/ used 82.5%"));
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_value_or_text_is_rejected_with_422() {
    let d = common::quiet().await;
    let admin = http(&d, Some(d.admin_token()));
    let long = "x".repeat(501);
    for body in [
        json!({ "id": "disk", "state": "ok", "value": "many" }),
        json!({ "id": "disk", "state": "ok", "text": 5 }),
        json!({ "id": "disk", "state": "ok", "text": long }),
    ] {
        assert_eq!(post(&admin, body.clone()).await, 422, "{body}");
    }
    assert_eq!(
        post(&admin, json!({ "id": "disk", "state": "ok" })).await,
        200,
        "value and text are optional"
    );
}
