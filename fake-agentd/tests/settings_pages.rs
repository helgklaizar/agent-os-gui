//! Settings pages `GET`/`PATCH /v1/settings/{page}` (api.md section 5.7): rev, ETag, If-Match, merge patch, checks.

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn page_default_roundtrip_etag_and_merge_patch() {
    let r = rig().await;
    let got = r.call("GET", "/v1/settings/account", None).await;
    assert_eq!(got.status, 200, "{}", text(&got));
    assert_eq!(got.json()["rev"], 0, "the page is not in the DB: rev = 0");
    assert_eq!(got.header("etag"), Some("\"0\""));
    let saved = r
        .call_if(
            "PATCH",
            "/v1/settings/account",
            0,
            json!({"name": "Anna", "language": "en"}),
        )
        .await;
    assert_eq!(saved.status, 200, "{}", text(&saved));
    assert_eq!(saved.json()["rev"], 1);
    assert_eq!(saved.header("etag"), Some("\"1\""));
    let again = r.get("/v1/settings/account").await;
    assert_eq!(
        (again["name"].as_str(), again["language"].as_str()),
        (Some("Anna"), Some("en"))
    );
    // merge patch: null removes a key, the rest stay
    let cut = r
        .call_if("PATCH", "/v1/settings/account", 1, json!({"name": null}))
        .await;
    assert_eq!(cut.status, 200);
    let doc = r.get("/v1/settings/account").await;
    assert!(doc.get("name").is_none(), "{doc}");
    assert_eq!(doc["language"], "en");
    assert_eq!(doc["rev"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn page_patch_conditions() {
    let r = rig().await;
    let none = r
        .call("PATCH", "/v1/settings/account", Some(json!({"name": "a"})))
        .await;
    assert_eq!(
        (none.status, code(&none)),
        (428, "precondition_required".into())
    );
    let stale = r
        .call_if("PATCH", "/v1/settings/account", 7, json!({"name": "a"}))
        .await;
    assert_eq!(
        (stale.status, code(&stale)),
        (412, "precondition_failed".into())
    );
    let req = Req::new("PATCH", "/v1/settings/account")
        .token(&r.admin)
        .header("If-Match", "abc")
        .json(json!({"name": "a"}));
    assert_eq!(
        send(r.d.socket_path(), req).await.status,
        400,
        "not a number"
    );
    assert_eq!(
        r.rev_of("/v1/settings/account").await,
        0,
        "refusals change nothing"
    );
    let body_not_object = r
        .call_if("PATCH", "/v1/settings/account", 0, json!([1]))
        .await;
    assert_eq!(body_not_object.status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn page_unknown_and_read_only() {
    let r = rig().await;
    assert_eq!(r.call("GET", "/v1/settings/nope", None).await.status, 404);
    let about = r.get("/v1/settings/about").await;
    assert!(about["core_version"].is_string(), "{about}");
    assert!(about["api"]["major"].is_number());
    assert!(about.get("capabilities").is_some() && about.get("db_version").is_some());
    assert_eq!(about["db_version"], 10, "mock DB version");
    let sec = r.get("/v1/settings/security").await;
    assert_eq!(sec["disk_encryption"], "unknown");
    assert_eq!(sec["secrets_present"], false);
    for page in ["about", "security"] {
        let w = r
            .call_if("PATCH", &format!("/v1/settings/{page}"), 0, json!({"x": 1}))
            .await;
        assert_eq!(w.status, 403, "{page}");
        assert_eq!(code(&w), "permission_denied");
        assert_eq!(w.json()["error"]["details"]["reason"], "read_only");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn page_scopes() {
    let r = rig().await;
    let read = r.token(Scope::Read, Kind::External);
    let chat = r.token(Scope::Chat, Kind::External);
    let get = r.call_as(&read, "GET", "/v1/settings/account", None).await;
    assert_eq!(get.status, 200);
    for t in [&read, &chat] {
        let w = r
            .call_as(
                t,
                "PATCH",
                "/v1/settings/account",
                Some(json!({"name": "x"})),
            )
            .await;
        assert_eq!((w.status, code(&w)), (403, "insufficient_scope".into()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn page_rejects_secret_values_but_allows_refs() {
    let r = rig().await;
    let bad = r
        .patch_page(
            "account",
            json!({"api_token": "sk-FIXTURE", "nested": {"password": "p"}}),
        )
        .await;
    assert_eq!(bad.status, 422);
    let f = fields(&bad);
    assert!(
        f.contains(&"api_token".to_string()) && f.contains(&"nested.password".to_string()),
        "{f:?}"
    );
    assert!(
        !text(&bad).contains("sk-FIXTURE"),
        "the value is not echoed in the error"
    );
    let ok = r
        .patch_page("account", json!({"token_ref": "provider.x"}))
        .await;
    assert_eq!(ok.status, 200, "{}", text(&ok));
    let big = r
        .patch_page("account", json!({"blob": "x".repeat(300_000)}))
        .await;
    assert!(matches!(big.status, 413 | 422), "{}", big.status);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_upsert_survives_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let r = rig_db(&db).await;
    let w = r.patch_page("account", json!({"name": "Anna"})).await;
    assert_eq!(w.status, 200);
    let w = r
        .patch_page("personas", json!({"balance": {"max_parallel": 3}}))
        .await;
    assert_eq!(w.status, 200);
    drop(r);
    let r = rig_db(&db).await;
    let a = r.get("/v1/settings/account").await;
    assert_eq!(
        (a["name"].as_str(), a["rev"].as_i64()),
        (Some("Anna"), Some(1))
    );
    let p = r.get("/v1/settings/personas").await;
    assert_eq!(p["balance"]["max_parallel"], 3);
    assert_eq!(p["balance"]["fallback"], "local", "the default value stays");
}
