//! A6-12: the `about` page is read-only: core, API and DB versions; writes are rejected.

mod common;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_about_reports_versions_and_is_read_only() {
    let e = Env::start().await;
    let (st, v) = e.read.get("/settings/about").await;
    assert_eq!(st, 200, "{v}");
    assert!(
        v["core_version"].as_str().is_some_and(|s| !s.is_empty()),
        "{v}"
    );
    assert_eq!(v["api"]["major"], 1);
    assert!(v["api"]["minor"].is_u64());
    assert_eq!(
        v["db_version"],
        agentd_db::MIGRATIONS.len() as i64,
        "DB version = number of migrations"
    );
    assert!(
        v["capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c == "if-match"))
    );
    let (st, v) = e
        .admin
        .patch_if_match(
            "/settings/about",
            json!({"core_version": "9"}),
            v["rev"].as_i64().unwrap_or(0),
        )
        .await;
    assert_eq!(st, 403, "{v}");
    let (_, again) = e.read.get("/settings/about").await;
    assert_ne!(again["core_version"], "9");
}
