//! `BarCluster` against `fake-agentd` over a real socket: frames, drop, one source, quotas from the DB.

mod bar_rig;
mod common;

use std::time::{Duration, Instant};

use bar_rig::*;
use common::send;
use controllers::controllers::metrics::MetricKind;
use controllers::level::{Level, load_level};
use core_client::http::Request;
use fake_agentd::Config;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn bar_syncs_with_fake_agentd() {
    let mut r = rig(Config::temp()
        .manual_clock()
        .bar_sys_period(Duration::from_secs(2))
        .bar_seed(5))
    .await;
    r.pump("snapshots", loaded).await;
    r.d.advance(Duration::from_secs(2));
    let before = disk(&r.cluster);
    r.pump("bar.sys frame after a tick", |c| disk(c) != before)
        .await;
    let v = r.cluster.view();
    // values and colours in the bar state match the frame that came over the socket
    for m in &v.metrics.items {
        let value = m.value.map(|x| x as f64).unwrap_or(f64::NAN);
        if m.kind != MetricKind::Temp {
            assert_eq!(m.level, load_level(value), "{:?}", m.kind);
        }
        assert!(m.value.is_some(), "{:?}", m);
    }
    let cl1 = &v.quotas.items[0];
    assert_eq!(
        (
            cl1.slug.as_str(),
            cl1.remaining_pct,
            cl1.rate_text.as_deref()
        ),
        ("cl1", Some(38), Some("1.4×"))
    );
    assert_eq!(cl1.remaining_level, Level::Warn);
    assert_eq!(v.quotas.items[1].remaining_pct, Some(82));
    assert_eq!(v.quotas.items[2].load_pct, Some(22));
    assert_eq!(v.bar.weather.temp, "24°");
    assert_eq!(v.bar.weather.icon, "weather-clear-symbolic");
    r.c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn bar_survives_ws_drop() {
    let mut r = rig(Config::temp()
        .manual_clock()
        .bar_sys_period(Duration::from_secs(2)))
    .await;
    r.pump("snapshots", loaded).await;
    r.d.drop_ws();
    r.d.advance(Duration::from_secs(5));
    r.pump("banner and grey indicators", |c| {
        let v = c.view();
        v.bar.offline && v.metrics.no_data && v.quotas.no_data
    })
    .await;
    r.d.restore();
    r.pump("data back after reconnecting", |c| {
        let v = c.view();
        !v.bar.offline && !v.metrics.no_data && !v.quotas.no_data && v.quotas.items.len() == 3
    })
    .await;
    r.c.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn bar_single_source_down() {
    let mut r = rig(Config::temp()
        .manual_clock()
        .bar_sys_period(Duration::from_secs(2)))
    .await;
    r.pump("snapshots", loaded).await;
    r.cluster.tick(r.d.now_unix(), Instant::now());
    let before = r.cluster.view();
    r.d.no_data("bar.weather");
    r.pump("weather without data", |c| {
        c.view().bar.weather.level == Level::NoData
    })
    .await;
    let after = r.cluster.view();
    assert_eq!(
        (
            after.metrics,
            after.quotas,
            after.bar.time,
            after.bar.date,
            after.bar.offline
        ),
        (
            before.metrics,
            before.quotas,
            before.bar.time,
            before.bar.date,
            before.bar.offline
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_reads_from_db_through_api() {
    let mut r = rig(Config::temp().no_ticker()).await;
    r.pump("snapshots", loaded).await;
    // change the table and publish: the frame must match the DB row and `GET /v1/quotas`
    let reset = r.d.now_unix() + 40 * 60;
    let w = r.d.db().write(|w| {
        w.execute("UPDATE quotas SET used = 0.9, rate = 2.5, reset_at = ?1 WHERE account_id = (SELECT id FROM accounts WHERE slug = 'cl2')", [reset])
    });
    assert!(w.is_ok(), "{w:?}");
    assert!(r.d.publish_tables().is_ok());
    r.pump("new cl2 balance", |c| {
        c.view()
            .quotas
            .items
            .get(1)
            .is_some_and(|q| q.remaining_pct == Some(10))
    })
    .await;
    r.cluster.tick(r.d.now_unix(), Instant::now());
    let cl2 = r.cluster.view().quotas.items[1].clone();
    let (used, rate, reset_db): (f64, f64, i64) = r
        .d
        .db()
        .reader()
        .and_then(|c| Ok(c.query_row("SELECT used, rate, reset_at FROM quotas WHERE account_id = (SELECT id FROM accounts WHERE slug = 'cl2')", [], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))?))
        .unwrap_or((-1.0, -1.0, -1));
    assert_eq!((used, rate, reset_db), (0.9, 2.5, reset));
    assert_eq!(
        (cl2.remaining_pct, cl2.rate_text.as_deref(), cl2.rate_level),
        (Some(10), Some("2.5×"), Level::Bad)
    );
    assert!(cl2.tooltip.contains("Resets in 40 min"), "{}", cl2.tooltip);
    let api = send(&r.http, Request::get("/v1/quotas")).await;
    let body: Value = serde_json::from_slice(&api.body).unwrap_or(Value::Null);
    let row = body
        .as_array()
        .and_then(|a| a.iter().find(|i| i["slug"] == "cl2"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        (
            row["used"].clone(),
            row["rate"].clone(),
            row["reset_at"].clone()
        ),
        (json!(0.9), json!(2.5), json!(reset))
    );
    // ETA 0.1 / (2.5 / 5) h = 12 min before the reset in 40 min
    assert!(
        cl2.tooltip.contains("Runs out in 12 min"),
        "{}",
        cl2.tooltip
    );
    r.c.shutdown();
}
