//! A7-04: `bar.sys` topic: `{cpu, gpu, ram, disk, temp}` (percent 0-100, temperature in °C), 2 s period, `snapshot` and `delta`
//! are identical; no sensor means `null`, never a fake value. A `read` token can subscribe, a foreign one cannot.

mod common;

use std::time::Duration;

use common::Env;
use core_client::api::FrameKind;
use serde_json::Value;

fn check_shape(d: &Value) {
    for k in ["cpu", "gpu", "ram", "disk", "temp"] {
        assert!(d.get(k).is_some(), "missing field {k}: {d}");
        if k != "temp" && !d[k].is_null() {
            let v = d[k].as_f64().unwrap_or(-1.0);
            assert!((0.0..=100.0).contains(&v), "{k} = {v}");
        }
    }
    assert!(d.as_object().unwrap().len() == 5, "extra fields: {d}");
}

#[tokio::test(flavor = "multi_thread")]
async fn bar_sys_snapshot_and_delta_have_the_same_shape() {
    let e = Env::start().await;
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.sys");
    check_shape(&snap);
    let f = ws.until(Duration::from_secs(8), |f| {
        f.topic == "bar.sys" && f.kind == FrameKind::Delta
    });
    check_shape(&f.last().unwrap().data);
    if cfg!(target_os = "linux") {
        assert!(
            snap["ram"].is_number() && snap["disk"].is_number(),
            "on Linux memory and disk are known: {snap}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bar_sys_period_is_about_two_seconds() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("bar.sys");
    let t0 = std::time::Instant::now();
    ws.until(Duration::from_secs(8), |f| f.kind == FrameKind::Delta);
    ws.until(Duration::from_secs(8), |f| f.kind == FrameKind::Delta);
    let dt = t0.elapsed();
    assert!(
        dt >= Duration::from_millis(1500) && dt < Duration::from_secs(7),
        "two frames in {dt:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sys_period_2s() {
    // Five consecutive frames: each interval is about 2 s (metric sampling is part of the period), no shorter than 1.5 s and no longer than 4 s.
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("bar.sys");
    let mut at = Vec::new();
    for _ in 0..5 {
        ws.until(Duration::from_secs(10), |f| f.kind == FrameKind::Delta);
        at.push(std::time::Instant::now());
    }
    for w in at.windows(2).skip(1) {
        let gap = w[1] - w[0];
        assert!(
            gap >= Duration::from_millis(1500) && gap <= Duration::from_secs(4),
            "interval between frames {gap:?}"
        );
    }
    let mean = (at[4] - at[1]) / 3;
    assert!(
        mean >= Duration::from_millis(1800) && mean <= Duration::from_millis(2600),
        "mean period {mean:?}"
    );
}
