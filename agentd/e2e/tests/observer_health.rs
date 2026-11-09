//! A4-08 (end-to-end): an unavailable observer puts `/health` into `degraded` (docs/reference/api.md, section 5.4).

mod common;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn observer_unavailable_health_degraded() {
    let e = Env::start().await;
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "ok", "{h}");
    let (st, v) = e
        .admin
        .post("/_test/observer", json!({"available": false}))
        .await;
    assert_eq!(st, 200, "{v}");
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "degraded", "{h}");
    let (st, _) = e
        .admin
        .post("/_test/observer", json!({"available": true}))
        .await;
    assert_eq!(st, 200);
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "ok", "{h}");
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_test_route_needs_admin() {
    let e = Env::start().await;
    for api in [&e.chat, &e.agent, &e.read] {
        let (st, _) = api
            .post("/_test/observer", json!({"available": false}))
            .await;
        assert_eq!(st, 403);
    }
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "ok");
}
