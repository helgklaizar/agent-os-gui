//! Bar data and frames in `fake-agentd`: `bar.sys` wandering, snapshots from tables, "no data", disconnect, hostile text.

mod bar_fake_common;
mod common;

use std::time::Duration;

use bar_fake_common::snapshot_of;
use common::{client, next_frame, start, wait_state};
use core_client::{ConnState, CoreEvent};
use fake_agentd::Config;
use serde_json::{Value, json};

const PERIOD: Duration = Duration::from_secs(2);

async fn sys_series(seed: u64, ticks: usize) -> Vec<Value> {
    let d = start(
        Config::temp()
            .manual_clock()
            .bar_sys_period(PERIOD)
            .bar_seed(seed),
    )
    .await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.sys").is_ok());
    let snap = next_frame(&rx, "bar.sys").await;
    assert!(snap.is_ok(), "bar.sys snapshot: {snap:?}");
    let mut out = Vec::new();
    for _ in 0..ticks {
        d.advance(PERIOD);
        match next_frame(&rx, "bar.sys").await {
            Ok(f) => out.push(f.data),
            Err(e) => panic!("{e}"),
        }
    }
    c.shutdown();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_sys_walk_is_reproducible_per_seed() {
    let a = sys_series(7, 8).await;
    let b = sys_series(7, 8).await;
    let other = sys_series(8, 8).await;
    assert_eq!(a, b, "same seed, same frames");
    assert_ne!(a, other, "different seed, different frames");
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_sys_walk_moves_every_metric_in_range() {
    let frames = sys_series(3, 30).await;
    for key in ["cpu", "gpu", "ram", "disk", "temp"] {
        let vals: Vec<i64> = frames.iter().filter_map(|f| f[key].as_i64()).collect();
        assert_eq!(vals.len(), frames.len(), "{key} is in every frame");
        assert!(
            vals.windows(2).all(|w| w[0] != w[1]),
            "{key}: every frame differs from the previous one: {vals:?}"
        );
        assert!(
            vals.windows(2).all(|w| (w[0] - w[1]).abs() <= 5),
            "{key}: the wander step is at most 5: {vals:?}"
        );
        let (lo, hi) = if key == "temp" { (30, 90) } else { (0, 100) };
        assert!(
            vals.iter().all(|v| (lo..=hi).contains(v)),
            "{key}: {vals:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_quota_fixtures_from_tables() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let f = snapshot_of(&c, "bar.quota").await;
    let items = f.data["items"].as_array().cloned().unwrap_or_default();
    let by = |slug: &str| {
        items
            .iter()
            .find(|i| i["slug"] == slug)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(items.len(), 3, "cl1, cl2, ol1: {:?}", f.data);
    let (cl1, cl2, ol1) = (by("cl1"), by("cl2"), by("ol1"));
    assert_eq!(
        (cl1["kind"].clone(), cl1["window_h"].clone()),
        (json!("api_key"), json!(5.0))
    );
    assert_eq!(
        (cl1["used"].clone(), cl1["rate"].clone()),
        (json!(0.62), json!(1.4))
    );
    assert_eq!(
        (cl2["used"].clone(), cl2["rate"].clone()),
        (json!(0.18), json!(0.4))
    );
    assert_eq!(ol1["kind"], json!("local"));
    assert_eq!(
        (ol1["window_h"].clone(), ol1["load"].clone()),
        (json!(0.0), json!(0.22))
    );
    assert!(
        ol1["used"].is_null() && ol1["rate"].is_null(),
        "a local model has no limit"
    );
    assert!(
        cl1["usage"]["tokens_in"].as_i64().is_some_and(|t| t > 0),
        "usage: {cl1}"
    );
    assert!(
        cl1["limits"]["requests_per_min"]["limit"]
            .as_i64()
            .is_some_and(|t| t > 0),
        "limits: {cl1}"
    );
    let now = d.now_unix();
    let reset = cl1["reset_at"].as_i64().unwrap_or(0);
    let eta = cl1["eta_at"].as_i64().unwrap_or(0);
    assert!(
        eta > now && eta < reset,
        "cl1 ETA is before the reset: eta {eta}, reset {reset}"
    );
    assert!(
        cl2["eta_at"].is_null(),
        "cl2 usage is below the allowed: no forecast"
    );
    c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_unknown_limit_row_has_nulls_and_usage_only() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.quota").is_ok());
    assert!(next_frame(&rx, "bar.quota").await.is_ok());
    let now = d.now_unix();
    let w = d.db().write(|w| {
        w.execute("INSERT INTO accounts(slug, type, label, status, created_at) VALUES ('cl3', 'anthropic', 'Claude · key-3', 'connected', ?1)", [now])?;
        w.execute("INSERT INTO quotas(account_id, kind, usage_tokens, usage_cost, updated_at) SELECT id, 'api_key', 900, 0.03, ?1 FROM accounts WHERE slug = 'cl3'", [now])
    });
    assert!(w.is_ok(), "{w:?}");
    assert!(d.publish_tables().is_ok());
    let f = match next_frame(&rx, "bar.quota").await {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    let items = f.data["items"].as_array().cloned().unwrap_or_default();
    let cl3 = items
        .iter()
        .find(|i| i["slug"] == "cl3")
        .cloned()
        .unwrap_or(Value::Null);
    assert!(
        cl3["window_h"].is_null() && cl3["used"].is_null() && cl3["rate"].is_null(),
        "{cl3}"
    );
    assert_eq!(cl3["usage"]["tokens_in"], json!(900));
    assert!(cl3["limits"].is_null(), "no limits: {cl3}");
    c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_weather_and_indicators_snapshots() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let w = snapshot_of(&c, "bar.weather").await;
    for key in ["temp_c", "icon", "text", "place", "updated_at"] {
        assert!(
            !w.data[key].is_null(),
            "bar.weather has no {key}: {}",
            w.data
        );
    }
    let i = snapshot_of(&c, "bar.indicators").await;
    let ids: Vec<String> = i.data["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for want in [
        "cpu",
        "gpu",
        "ram",
        "disk",
        "temp",
        "quota.cl1",
        "quota.cl2",
        "quota.ol1",
    ] {
        assert!(
            ids.iter().any(|x| x == want),
            "no indicator {want}: {ids:?}"
        );
    }
    assert!(
        i.data["items"][0]["state"] == json!("unknown"),
        "initial state unknown"
    );
    c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_bar_no_data_and_ws_drop_from_test() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    assert!(c.subscribe("bar.weather").is_ok());
    assert!(next_frame(&rx, "bar.weather").await.is_ok());
    d.no_data("bar.weather");
    let f = match next_frame(&rx, "bar.weather").await {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    assert!(
        f.data["temp_c"].is_null() && f.data["icon"].is_null(),
        "{}",
        f.data
    );
    d.drop_ws();
    assert!(
        common::wait_event(
            &rx,
            "drop",
            |e| matches!(e, CoreEvent::State(s) if *s != ConnState::Connected)
        )
        .await
        .is_ok()
    );
    d.restore();
    assert!(wait_state(&rx, ConnState::Connected).await.is_ok());
    c.shutdown();
}
