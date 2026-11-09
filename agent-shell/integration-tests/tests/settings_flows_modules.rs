//! End-to-end Settings scenarios against `fake-agentd`: module installation and a module URL via the observer.

mod settings_common;

use controllers::SettingsPage;
use controllers::i18n::tr;
use controllers::settings_types::Action;
use settings_common::*;

#[tokio::test(flavor = "multi_thread")]
async fn module_install_flow() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Modules).await;
    assert!(
        r.titles().iter().any(|t| t == "Weather"),
        "the market shows the catalog: {:?}",
        r.titles()
    );
    r.act(Action::ModuleInstall("weather-card".into())).await;
    let list = r.http.list_modules().await.expect("modules");
    assert!(
        list.iter()
            .any(|m| m["id"] == "weather-card" && m["installed"] == true),
        "installed"
    );
    // setup by the manifest question
    assert!(
        r.field("m:weather-card:cfg:city").is_some(),
        "manifest question is a field"
    );
    r.edit("m:weather-card:cfg:city", "Haifa").await;
    r.edit("m:weather-card:placement", "panel").await;
    r.save().await;
    let m = r
        .http
        .list_modules()
        .await
        .expect("modules")
        .into_iter()
        .find(|m| m["id"] == "weather-card")
        .expect("module");
    assert_eq!(
        (m["config"]["city"].as_str(), m["placement"].as_str()),
        (Some("Haifa"), Some("panel"))
    );
    // enabling and disabling
    r.act(Action::ModuleToggle("weather-card".into())).await;
    let m = r
        .http
        .list_modules()
        .await
        .expect("modules")
        .into_iter()
        .find(|m| m["id"] == "weather-card")
        .expect("module");
    assert_eq!(m["enabled"], false);
    // removal needs confirmation, the module returns to the catalog
    r.act(Action::ModuleRemove("weather-card".into())).await;
    assert!(r.view().confirm.is_some());
    r.msg(controllers::settings_view::SettingsMsg::Confirm(true))
        .await;
    let m = r
        .http
        .list_modules()
        .await
        .expect("modules")
        .into_iter()
        .find(|m| m["id"] == "weather-card")
        .expect("in the catalog");
    assert_eq!(m["installed"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn module_custom_url_goes_through_observer() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Modules).await;
    let installed_ids =
        |l: &Vec<serde_json::Value>| l.iter().filter(|m| m["installed"] == true).count();
    let base = installed_ids(&r.http.list_modules().await.expect("modules"));
    // the observer rejected
    r.edit("m:url", "https://modules.example.test/evil.json")
        .await;
    r.act(Action::ModuleInstallUrl).await;
    assert_eq!(r.banner(), Some(tr("set-err-invalid")));
    assert_eq!(
        installed_ids(&r.http.list_modules().await.expect("modules")),
        base,
        "the rejected module is not installed"
    );
    // waiting for a decision
    r.edit("m:url", "https://modules.example.test/ask-notes-panel.json")
        .await;
    r.act(Action::ModuleInstallUrl).await;
    assert_eq!(r.banner(), Some(tr("set-module-pending")));
    assert_eq!(
        installed_ids(&r.http.list_modules().await.expect("modules")),
        base,
        "not installed without a decision"
    );
    // the check passed
    r.edit("m:url", "https://modules.example.test/notes-panel.json")
        .await;
    r.act(Action::ModuleInstallUrl).await;
    assert_eq!(
        installed_ids(&r.http.list_modules().await.expect("modules")),
        base + 1
    );
    // the client does not send non-https at all
    r.edit("m:url", "http://modules.example.test/quick-btn.json")
        .await;
    let n = r.effects.len();
    r.act(Action::ModuleInstallUrl).await;
    assert!(r.field("m:url").is_some_and(|f| f.error.is_some()));
    assert!(
        r.effects[n..]
            .iter()
            .all(|e| !format!("{e:?}").contains("InstallModule")),
        "the request did not go out"
    );
}
