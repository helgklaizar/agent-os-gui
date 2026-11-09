//! Save failure on every page and on list pages, `If-Match` conflict.

mod settings_common;

use controllers::SettingsPage;
use controllers::i18n::tr;
use controllers::settings_fields::page_specs;
use controllers::settings_types::{Action, Secret};
use controllers::settings_view::SettingsMsg;
use core_client::http::Request;
use serde_json::json;
use settings_common::*;

/// A page that has a document field, and the field edited in the test.
const DOC_PAGES: [(SettingsPage, &str); 7] = [
    (SettingsPage::Account, "account"),
    (SettingsPage::Personas, "personas"),
    (SettingsPage::Models, "models"),
    (SettingsPage::Security, "account"),
    (SettingsPage::Schedule, "schedule"),
    (SettingsPage::Permissions, "permissions"),
    (SettingsPage::Tunnels, "tunnels"),
];

#[tokio::test(flavor = "multi_thread")]
async fn settings_save_failure_on_each_page() {
    for (page, doc) in DOC_PAGES {
        let mut r = Rig::start().await;
        r.open(page).await;
        let spec = page_specs(page, &r.c_model())
            .into_iter()
            .find(|s| s.doc == doc)
            .unwrap_or_else(|| panic!("{page:?}: no document field {doc}"));
        let current = r.field(&spec.id).map(|f| f.value).unwrap_or_default();
        let new = sample(&spec, &current);
        r.edit(&spec.id, &new).await;
        r.d.fail_next(&format!("PATCH /v1/settings/{doc}"), 500, 1);
        r.save().await;
        let v = r.view();
        assert_eq!(
            v.banner.map(|b| b.text),
            Some(tr("set-err-unavailable")),
            "{page:?}: the error is visible in the dialog"
        );
        assert!(
            v.dirty && !v.saving,
            "{page:?}: the input is not lost, a retry is possible"
        );
        assert_eq!(
            r.field(&spec.id).map(|f| f.value),
            Some(new.clone()),
            "{page:?}: the value stayed"
        );
        r.save().await;
        assert!(
            !r.view().dirty,
            "{page:?}: the retry was saved: {:?}",
            r.banner()
        );
        let page_now = r.http.settings_page(doc).await.expect("page");
        assert!(page_now.rev >= 1, "{page:?}: the write reached the core");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_save_failure_on_entity_pages() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Personas).await;
    r.edit("p:1:name", "Renamed").await;
    r.d.fail_next("PATCH /v1/personas/1", 503, 1);
    r.save().await;
    assert_eq!(r.banner(), Some(tr("set-err-unavailable")));
    assert_eq!(
        r.field("p:1:name").map(|f| f.value).as_deref(),
        Some("Renamed")
    );
    r.save().await;
    assert!(!r.view().dirty, "{:?}", r.banner());
    r.select(SettingsPage::Models).await;
    r.msg(SettingsMsg::EditSecret {
        id: "s:account.1".into(),
        value: Secret::new("k"),
    })
    .await;
    r.d.fail_next("PUT /v1/secrets/account.1", 500, 1);
    r.save().await;
    assert!(r.view().dirty, "the secret is not lost on failure");
    r.save().await;
    assert!(!r.view().dirty);
    r.select(SettingsPage::Tunnels).await;
    r.edit("t:new:addr", "10.1.1.1:22").await;
    r.d.fail_next("POST /v1/tunnels", 500, 1);
    r.act(Action::TunnelAdd).await;
    assert_eq!(r.banner(), Some(tr("set-err-unavailable")));
    assert_eq!(
        r.field("t:new:addr").map(|f| f.value).as_deref(),
        Some("10.1.1.1:22"),
        "on failure the new-entry form keeps what was entered"
    );
    r.act(Action::TunnelAdd).await;
    assert_eq!(
        r.field("t:new:addr").map(|f| f.value).as_deref(),
        Some(""),
        "after success the form is cleared"
    );
    assert!(
        r.titles().iter().any(|t| t == "wireguard"),
        "tunnel created by the retry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_concurrent_write_conflict() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Account).await;
    r.edit("d:account:name", "Mine").await;
    // another client (a phone) managed to change the page
    let page = r.http.settings_page("account").await.expect("page");
    let other = r
        .http
        .patch_settings_page("account", page.rev, &json!({"name": "Phone"}))
        .await;
    assert!(other.is_ok(), "{other:?}");
    r.save().await;
    assert_eq!(
        r.banner(),
        Some(tr("set-err-stale")),
        "412 is understandable to the user"
    );
    assert_eq!(
        r.field("d:account:name").map(|f| f.value).as_deref(),
        Some("Mine"),
        "the input stayed"
    );
    assert!(r.view().dirty);
    r.save().await;
    assert!(
        !r.view().dirty,
        "the retry with a new rev went through: {:?}",
        r.banner()
    );
    let now = r.http.settings_page("account").await.expect("page");
    assert_eq!(now.doc["name"], "Mine");
    // without If-Match 428, with a stale one 412
    let no_header = r
        .http
        .call(Request::patch("/v1/settings/account", json!({"name": "X"})))
        .await;
    assert_eq!(status(&no_header), Some(428));
    let stale = r
        .http
        .patch_settings_page("account", 1, &json!({"name": "X"}))
        .await;
    assert_eq!(status(&stale), Some(412));
}
