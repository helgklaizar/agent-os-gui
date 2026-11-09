//! `balance.fallback` in `BarCluster` against `fake-agentd`: reading, API refusals, update.

mod bar_rig;
mod common;

use std::time::Instant;

use bar_rig::*;
use common::send;
use controllers::bar_cluster::BarCluster;
use core_client::http::Request;
use fake_agentd::Config;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn bar_balance_fallback_roundtrip() {
    let mut r = rig(Config::temp().no_ticker()).await;
    let refresh = r.pump("snapshots", loaded).await;
    assert!(refresh, "after connecting, balance.fallback must be read");
    r.read_fallback().await;
    r.cluster.tick(r.d.now_unix(), Instant::now());
    let tip = |c: &BarCluster| c.view().quotas.items[0].tooltip.clone();
    assert!(
        tip(&r.cluster).contains("Then: local model"),
        "{}",
        tip(&r.cluster)
    );
    // a foreign client changes the setting through the core API
    let page = r.http.personas().await;
    let rev = page.map(|p| p.rev).unwrap_or(-1);
    let patch = |rev: i64, v: &str| {
        Request::patch(
            "/v1/settings/personas",
            json!({ "balance": { "fallback": v } }),
        )
        .header("If-Match", &rev.to_string())
    };
    let stale = send(&r.http, patch(rev + 5, "wait")).await;
    assert_eq!(stale.status, 412, "stale rev rejected");
    let bad = send(&r.http, patch(rev, "nonsense")).await;
    assert_eq!(bad.status, 422, "unknown value rejected");
    let missing = send(
        &r.http,
        Request::patch(
            "/v1/settings/personas",
            json!({ "balance": { "fallback": "wait" } }),
        ),
    )
    .await;
    assert_eq!(missing.status, 428, "without If-Match");
    assert!(
        tip(&r.cluster).contains("Then: local model"),
        "refusals change nothing"
    );
    let ok = send(&r.http, patch(rev, "wait")).await;
    assert_eq!(ok.status, 200);
    let refreshed = r.pump_until_refresh().await;
    assert!(
        refreshed,
        "a settings frame page=personas requires re-reading the fallback"
    );
    r.read_fallback().await;
    assert!(
        tip(&r.cluster).contains("Then: wait for reset"),
        "{}",
        tip(&r.cluster)
    );
    r.c.shutdown();
}
