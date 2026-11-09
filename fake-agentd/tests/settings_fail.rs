//! Mock side: failure injection `POST /v1/_test/fail` and `Fake::fail_next`.

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn fail_next_returns_status_without_touching_data() {
    let r = rig().await;
    let inj = r
        .call(
            "POST",
            "/v1/_test/fail",
            Some(json!({"route": "PATCH /v1/settings/models", "status": 500, "times": 1})),
        )
        .await;
    assert_eq!(inj.status, 204, "{}", text(&inj));
    let w = r
        .patch_page("models", json!({"context_budget": 1000}))
        .await;
    assert_eq!((w.status, code(&w)), (500, "internal".into()));
    assert!(w.json()["error"]["request_id"].is_string());
    assert_eq!(
        r.rev_of("/v1/settings/models").await,
        0,
        "data did not change"
    );
    let again = r
        .patch_page("models", json!({"context_budget": 1000}))
        .await;
    assert_eq!(again.status, 200, "the failure is one-shot");
    assert_eq!(r.get("/v1/settings/models").await["context_budget"], 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_next_statuses_times_and_reset() {
    let r = rig().await;
    for (status, want) in [
        (503, "core_down"),
        (422, "validation_failed"),
        (412, "precondition_failed"),
        (500, "internal"),
    ] {
        r.d.fail_next("PATCH /v1/settings/account", status, 2);
        for _ in 0..2 {
            let w = r.patch_page("account", json!({"name": "a"})).await;
            assert_eq!((w.status, code(&w)), (status, want.into()), "{status}");
        }
        assert_eq!(
            r.patch_page("account", json!({"name": "a"})).await.status,
            200,
            "after 2 times - normal"
        );
    }
    r.d.fail_next("GET /v1/tunnels", 503, 5);
    assert_eq!(r.call("GET", "/v1/tunnels", None).await.status, 503);
    assert_eq!(r.call("DELETE", "/v1/_test/fail", None).await.status, 204);
    assert_eq!(
        r.call("GET", "/v1/tunnels", None).await.status,
        200,
        "reset"
    );
    let bad = r
        .call(
            "POST",
            "/v1/_test/fail",
            Some(json!({"route": "GET /v1/tunnels", "status": 418})),
        )
        .await;
    assert_eq!(bad.status, 400, "other codes are not accepted");
    let bad = r
        .call(
            "POST",
            "/v1/_test/fail",
            Some(json!({"route": "nonsense", "status": 500})),
        )
        .await;
    assert_eq!(bad.status, 400);
    r.call(
        "POST",
        "/v1/_test/fail",
        Some(json!({"route": "GET /v1/tunnels", "status": 500})),
    )
    .await;
    assert_eq!(
        r.call("GET", "/v1/tunnels", None).await.status,
        500,
        "times defaults to 1"
    );
    assert_eq!(r.call("GET", "/v1/tunnels", None).await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_route_is_admin_and_exact() {
    let r = rig().await;
    let chat = r.token(Scope::Chat, Kind::External);
    let w = r
        .call_as(
            &chat,
            "POST",
            "/v1/_test/fail",
            Some(json!({"route": "GET /v1/tunnels", "status": 500})),
        )
        .await;
    assert_eq!(w.status, 403);
    r.d.fail_next("GET /v1/tunnels", 500, 1);
    assert_eq!(
        r.call("GET", "/v1/tunnels?x=1", None).await.status,
        500,
        "the query is not part of the key"
    );
    r.d.fail_next("GET /v1/tunnels", 500, 1);
    assert_eq!(
        r.call("GET", "/v1/accounts", None).await.status,
        200,
        "another route is not affected"
    );
    assert_eq!(r.call("GET", "/v1/tunnels", None).await.status, 500);
}
