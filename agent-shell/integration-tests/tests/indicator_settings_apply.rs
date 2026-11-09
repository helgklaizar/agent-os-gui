//! S8 on `fake-agentd`: changing the indicator setting reaches the bar within 500 ms.
//! Whole path: `PATCH /v1/settings/indicators` -> `settings` frame over WS -> page `GET` -> `IndicatorController`.
//! This does not measure the GTK bar rebuild (not done: needs GTK); it measures the whole path from the core to the bar controller.

mod common;

use std::time::{Duration, Instant};

use common::{client, http, send, wait_event};
use controllers::Controller;
use controllers::controllers::indicator::{IndicatorCfg, IndicatorController, Msg};
use core_client::CoreEvent;
use core_client::http::Request;
use fake_agentd::{Config, FakeAgentd};
use serde_json::{Value, json};

const LIMIT: Duration = Duration::from_millis(500);

fn cfgs(page: &Value) -> Vec<IndicatorCfg> {
    page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| IndicatorCfg {
            id: i["id"].as_str().unwrap_or_default().to_string(),
            enabled: i["enabled"].as_bool().unwrap_or(false),
            position: i["position"].as_i64().unwrap_or(0),
        })
        .collect()
}

fn visible(ctl: &IndicatorController) -> Vec<String> {
    ctl.view().visible.iter().map(|i| i.id.clone()).collect()
}

async fn page(c: &core_client::HttpClient) -> Value {
    let r = send(c, Request::get("/v1/settings/indicators")).await;
    assert_eq!(r.status, 200);
    r.json().expect("indicators page")
}

/// A `settings` delta about the indicators page (the subscription snapshot does not count).
fn is_indicators_delta(e: &CoreEvent) -> bool {
    matches!(e, CoreEvent::Frame(f) if f.topic == "settings" && f.data["page"] == "indicators")
}

async fn quiet() -> FakeAgentd {
    common::start(Config::temp().no_ticker()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_setting_change_reaches_bar_within_500ms() {
    let d = quiet().await;
    let core = client(&d);
    assert!(core.subscribe("settings").is_ok());
    let rx = core.events();
    let admin = http(&d, Some(d.admin_token()));

    let before = page(&admin).await;
    let mut ctl = IndicatorController::default();
    let t0 = Instant::now();
    ctl.update(Msg::Width(4000), t0);
    ctl.update(Msg::Config(cfgs(&before)), t0);
    let initial = visible(&ctl);
    assert!(initial.len() >= 3, "the table has indicators: {initial:?}");
    let g0 = ctl.view().generation;

    // turn off the first and put the last first
    let mut items = before["items"].as_array().cloned().unwrap_or_default();
    items[0]["enabled"] = json!(false);
    let n = items.len();
    for (i, it) in items.iter_mut().enumerate() {
        it["position"] = json!(if i == n - 1 { 0 } else { i + 1 });
    }
    let rev = before["rev"].as_i64().unwrap_or(0);

    let started = Instant::now();
    let r = send(
        &admin,
        Request::patch("/v1/settings/indicators", json!({ "items": items }))
            .header("If-Match", &rev.to_string()),
    )
    .await;
    assert_eq!(r.status, 200);
    let frame = wait_event(&rx, "settings/indicators delta", is_indicators_delta).await;
    assert!(frame.is_ok(), "{frame:?}");
    let after = page(&admin).await;
    assert_eq!(after["rev"].as_i64(), Some(rev + 1));
    ctl.update(Msg::Config(cfgs(&after)), Instant::now());
    let took = started.elapsed();

    assert!(took < LIMIT, "from PATCH to the bar rebuild: {took:?}");
    assert!(ctl.view().generation > g0, "the bar was rebuilt");
    let now = visible(&ctl);
    assert!(
        !now.contains(&initial[0]),
        "the disabled one is hidden: {now:?}"
    );
    assert_eq!(
        now.first(),
        initial.last(),
        "order from the setting: {now:?}"
    );
    assert_eq!(now.len(), initial.len() - 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_indicator_patch_changes_nothing_and_sends_no_frame() {
    let d = quiet().await;
    let core = client(&d);
    assert!(core.subscribe("settings").is_ok());
    let rx = core.events();
    let admin = http(&d, Some(d.admin_token()));
    let before = page(&admin).await;
    let rev = before["rev"].as_i64().unwrap_or(0);

    let r = send(
        &admin,
        Request::patch(
            "/v1/settings/indicators",
            json!({ "items": before["items"] }),
        )
        .header("If-Match", &(rev + 7).to_string()),
    )
    .await;
    assert_eq!(r.status, 412);
    assert_eq!(page(&admin).await, before, "the page did not change");
    let got = tokio::time::timeout(
        Duration::from_millis(300),
        wait_event(&rx, "delta", is_indicators_delta),
    )
    .await;
    assert!(got.is_err(), "a settings frame is not sent on refusal");
}
