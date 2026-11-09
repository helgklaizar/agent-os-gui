//! A7-07: the `cpu`, `ram`, `disk`, `temp` indicators are built from `bar.sys` metrics (thresholds 50/80, temperature 60/80); with no
//! sensor the state is `nodata`, not a fake value.

mod common;

use std::time::Duration;

use common::Env;

async fn state_of(e: &Env, id: &str) -> Option<String> {
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.indicators");
    snap["items"]
        .as_array()?
        .iter()
        .find(|i| i["id"] == id)
        .and_then(|i| i["state"].as_str().map(str::to_string))
}

#[tokio::test(flavor = "multi_thread")]
async fn indicators_sys_follow_metrics_and_mark_missing_sensors() {
    let e = Env::start().await;
    std::fs::write(e.home.join("sys-indicators.on"), "1").unwrap();
    // The first sample arrives immediately; indicators are polled every 5 s.
    let mut disk = None;
    for _ in 0..40 {
        disk = state_of(&e, "disk").await;
        if matches!(disk.as_deref(), Some("ok" | "warn" | "bad")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        matches!(disk.as_deref(), Some("ok" | "warn" | "bad")),
        "disk is always known: {disk:?}"
    );
    if cfg!(target_os = "linux") {
        let mut ram = None;
        for _ in 0..20 {
            ram = state_of(&e, "ram").await;
            if matches!(ram.as_deref(), Some("ok" | "warn" | "bad")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(
            matches!(ram.as_deref(), Some("ok" | "warn" | "bad")),
            "{ram:?}"
        );
    } else {
        assert_eq!(
            state_of(&e, "cpu").await.as_deref(),
            Some("nodata"),
            "no CPU sensor outside Linux"
        );
    }
}
