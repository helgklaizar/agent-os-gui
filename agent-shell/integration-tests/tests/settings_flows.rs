//! End-to-end Settings scenarios against `fake-agentd`: personas, tunnels, modules, files,
//! indicators, connections.

mod settings_common;

use controllers::SettingsPage;
use controllers::i18n::tr;
use controllers::settings_types::Action;
use controllers::settings_view::Row;
use serde_json::json;
use settings_common::*;

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_add_toggle_delete() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Tunnels).await;
    let before = r.titles().len();
    r.edit("t:new:type", "ssh").await;
    r.edit("t:new:addr", "<b>h</b>:22").await;
    r.edit("t:new:descr", "\" onmouseover=\"x").await;
    r.act(Action::TunnelAdd).await;
    assert_eq!(
        r.titles().len(),
        before + 1,
        "the tunnel appeared in the list: {:?}",
        r.banner()
    );
    let list = r.http.list_tunnels().await.expect("tunnels");
    let mine = list
        .iter()
        .find(|t| t["addr"] == "<b>h</b>:22")
        .expect("in the core");
    assert_eq!(
        mine["descr"], "\" onmouseover=\"x",
        "hostile strings saved literally"
    );
    let id = mine["id"].as_i64().expect("id");
    assert_eq!(mine["enabled"], false);
    r.act(Action::TunnelToggle(id)).await;
    let list = r.http.list_tunnels().await.expect("tunnels");
    assert!(
        list.iter().any(|t| t["id"] == id && t["enabled"] == true),
        "enabled"
    );
    // deletion goes through confirmation
    r.act(Action::TunnelDelete(id)).await;
    assert!(r.view().confirm.is_some());
    assert!(
        r.http
            .list_tunnels()
            .await
            .expect("tunnels")
            .iter()
            .any(|t| t["id"] == id),
        "before confirmation the tunnel is in place"
    );
    r.msg(controllers::settings_view::SettingsMsg::Confirm(true))
        .await;
    assert!(
        !r.http
            .list_tunnels()
            .await
            .expect("tunnels")
            .iter()
            .any(|t| t["id"] == id)
    );
    // "API from outside" and the state survive a restart
    r.edit("d:tunnels:api_external", "true").await;
    r.save().await;
    let mut r = r.restart().await;
    r.open(SettingsPage::Tunnels).await;
    assert_eq!(
        r.field("d:tunnels:api_external")
            .map(|f| f.value)
            .as_deref(),
        Some("true")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_files_roundtrip_and_only_allowed() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Files).await;
    let titles = r.titles();
    assert!(titles.contains(&"AGENT.md".to_string()), "{titles:?}");
    assert!(
        titles
            .iter()
            .all(|t| t == "AGENT.md" || t.starts_with("memory/")),
        "{titles:?}"
    );
    r.act(Action::FileOpen("AGENT.md".into())).await;
    let before = r.field("f:text").map(|f| f.value).expect("file text");
    r.edit("f:text", &format!("{before}\nedit")).await;
    r.save().await;
    assert!(!r.view().dirty, "{:?}", r.banner());
    let mut r = r.restart().await;
    r.open(SettingsPage::Files).await;
    r.act(Action::FileOpen("AGENT.md".into())).await;
    assert!(
        r.field("f:text").is_some_and(|f| f.value.ends_with("edit")),
        "the edit survived a restart"
    );
    // forbidden files: the core answers 403, the client does not reach it, the decoy secret is nowhere
    for bad in ["keys.env", "permissions.json", "memory/../keys.env"] {
        let resp = r.http.file_content(bad).await;
        assert_eq!(
            resp.err().and_then(|e| e.api().map(|a| a.status)),
            Some(403),
            "{bad}"
        );
    }
    let text = format!("{:?}{:?}", r.view(), r.c_model().files);
    assert!(!text.contains("FIXTURE-SECRET-DO-NOT-LEAK"));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_indicators_persist_order_and_enable() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Indicators).await;
    let before = r.titles();
    assert!(before.len() >= 3, "{before:?}");
    let first = before[0].clone();
    r.act(Action::IndicatorDown(first.clone())).await;
    r.act(Action::IndicatorToggle(before[1].clone())).await;
    r.save().await;
    assert!(!r.view().dirty, "{:?}", r.banner());
    let page = r.http.settings_page("indicators").await.expect("page");
    let items = page.doc["items"].as_array().cloned().unwrap_or_default();
    let ids: Vec<_> = items.iter().map(|i| value(&i["id"])).collect();
    assert_eq!(ids[1], first, "order saved in the core: {ids:?}");
    let toggled = items
        .iter()
        .find(|i| i["id"] == before[1].as_str())
        .expect("indicator");
    assert_eq!(toggled["enabled"], false);
    let mut r = r.restart().await;
    r.open(SettingsPage::Indicators).await;
    let after = r.titles();
    assert_eq!(after[1], first, "the order survived a restart");
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_unofficial_channel_label() {
    let mut r = Rig::start().await;
    r.http
        .create_account(json!({"type": "wa", "label": "WhatsApp Whapi"}))
        .await
        .expect("account");
    r.http
        .create_account(json!({"type": "tg", "label": "Telegram Work"}))
        .await
        .expect("account");
    r.open(SettingsPage::Connections).await;
    let tags = |title: &str| {
        r.view().rows.into_iter().find_map(|x| match x {
            Row::Item(i) if i.title == title => Some(i.tags),
            _ => None,
        })
    };
    assert_eq!(tags("WhatsApp Whapi"), Some(vec![tr("set-unofficial")]));
    assert_eq!(tags("Telegram Work"), Some(vec![]));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_persona_delete_resets_priority_and_rename_keeps_it() {
    let mut r = Rig::start().await;
    let created = r
        .http
        .create_persona(json!({"name": "Prio"}))
        .await
        .expect("persona");
    let id = created["id"].as_i64().expect("id");
    r.open(SettingsPage::Personas).await;
    r.edit("d:personas:balance.priority_persona_id", &id.to_string())
        .await;
    r.save().await;
    assert!(!r.view().dirty, "{:?}", r.banner());
    // renaming does not break the priority
    r.act(Action::PersonaSelect(id)).await;
    r.edit(&format!("p:{id}:name"), "Prio Renamed").await;
    r.save().await;
    let page = r.http.settings_page("personas").await.expect("page");
    assert_eq!(
        page.doc["balance"]["priority_persona_id"], id,
        "priority by id"
    );
    assert_eq!(
        r.field("d:personas:balance.priority_persona_id")
            .map(|f| f.value),
        Some(id.to_string())
    );
    // deletion: confirmation with a list, then the priority is reset by the core and visible in the dialog
    r.act(Action::PersonaDelete(id)).await;
    let confirm = r.view().confirm.expect("confirmation");
    assert!(
        confirm
            .lines
            .join("\n")
            .contains(&tr("set-confirm-persona-priority"))
    );
    r.msg(controllers::settings_view::SettingsMsg::Confirm(true))
        .await;
    let page = r.http.settings_page("personas").await.expect("page");
    assert!(
        page.doc["balance"]
            .get("priority_persona_id")
            .is_none_or(serde_json::Value::is_null),
        "{page:?}"
    );
    assert_eq!(
        r.field("d:personas:balance.priority_persona_id")
            .map(|f| f.value)
            .as_deref(),
        Some("")
    );
}
