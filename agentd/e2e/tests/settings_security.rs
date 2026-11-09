//! A6-12: `security` page is read-only: `ptrace_scope`, disk encryption, `secrets_present` (whether secrets exist, without
//! values or names).

mod common;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_security_is_computed_read_only_and_hides_secrets() {
    let e = Env::start().await;
    let (st, v) = e.read.get("/settings/security").await;
    assert_eq!(st, 200, "{v}");
    assert!(
        v["ptrace_scope"].is_null() || v["ptrace_scope"].is_u64(),
        "{v}"
    );
    assert_eq!(v["disk_encryption"], "unknown");
    assert_eq!(v["secrets_present"], false);
    let (st, _) = e
        .admin
        .put(
            "/secrets/provider.test",
            json!({"value": "secret-value-not-for-the-page"}),
        )
        .await;
    assert!(st == 200 || st == 201 || st == 204);
    let (_, v) = e.read.get("/settings/security").await;
    assert_eq!(v["secrets_present"], true);
    let text = v.to_string();
    assert!(
        !text.contains("secret-value") && !text.contains("provider.test"),
        "{text}"
    );
    let (st, v2) = e
        .admin
        .patch_if_match(
            "/settings/security",
            json!({"secrets_present": false}),
            v["rev"].as_i64().unwrap_or(0),
        )
        .await;
    assert_eq!(st, 403, "{v2}");
}
