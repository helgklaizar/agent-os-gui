//! `fake-agentd` scenarios for the bar: hostile text verbatim, ETA after reset.

mod bar_fake_common;
mod common;

use std::path::PathBuf;

use bar_fake_common::snapshot_of;
use common::{client, start, wait_state};
use core_client::ConnState;
use fake_agentd::{Config, FakeAgentd, Scenario};
use serde_json::{Value, json};

fn scenario(name: &str) -> Scenario {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fake-agentd/scenarios")
        .join(name);
    match Scenario::from_file(&path) {
        Ok(s) => s,
        Err(e) => panic!("scenario {name}: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_evil_label_scenario_delivers_text_verbatim() {
    let d: FakeAgentd = start(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .scenario(scenario("bar-evil-label.toml")),
    )
    .await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.quota").is_ok());
    // the step at_ms = 0 goes before the subscription or right after it: wait for the frame with the scenario title
    let got = common::wait_event(&rx, "quota with the scenario title", |e| {
        common::frame_of(e).is_some_and(|f| {
            f.topic == "bar.quota"
                && f.data["items"][0]["label"] == json!("<img src=x onerror=alert(1)>")
        })
    })
    .await;
    assert!(got.is_ok(), "{got:?}");
    c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_eta_is_null_when_after_reset() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let reset = d.now_unix() + 60 * 60;
    let w = d.db().write(|w| {
        w.execute("UPDATE quotas SET used = 0.62, rate = 1.4, reset_at = ?1 WHERE account_id = (SELECT id FROM accounts WHERE slug = 'cl1')", [reset])
    });
    assert!(w.is_ok(), "{w:?}");
    assert!(d.publish_tables().is_ok());
    let f = snapshot_of(&c, "bar.quota").await;
    let items = f.data["items"].as_array().cloned().unwrap_or_default();
    let cl1 = items
        .iter()
        .find(|i| i["slug"] == "cl1")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(cl1["rate"], json!(1.4));
    assert!(
        cl1["eta_at"].is_null(),
        "an ETA later than the reset is not given: {cl1}"
    );
    c.shutdown();
}
