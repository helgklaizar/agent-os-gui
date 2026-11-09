//! The shape of `GET /v1/settings/indicators` matches the core (api.md section 5.7: `config?` is optional; the core returns `{id, enabled, position}`).

mod s7_common;

use s7_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn indicators_without_config_omit_the_field() {
    let r = rig().await;
    let page = r.get("/v1/settings/indicators").await;
    let items = page["items"].as_array().cloned().unwrap_or_default();
    assert!(!items.is_empty(), "{page}");
    for it in &items {
        assert!(
            it.get("config").is_none(),
            "an empty config is not returned: {it}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_config_is_returned_once_set() {
    let r = rig().await;
    let page = r.get("/v1/settings/indicators").await;
    let rev = page["rev"].as_i64().unwrap_or(-1);
    let mut items = page["items"].as_array().cloned().unwrap_or_default();
    items[0]["config"] = json!({"warn": 80});
    let w = r
        .call_if(
            "PATCH",
            "/v1/settings/indicators",
            rev,
            json!({"items": items}),
        )
        .await;
    assert_eq!(w.status, 200, "{}", text(&w));
    let after = r.get("/v1/settings/indicators").await;
    assert_eq!(after["items"][0]["config"], json!({"warn": 80}), "{after}");
    assert!(after["items"][1].get("config").is_none(), "{after}");
}
