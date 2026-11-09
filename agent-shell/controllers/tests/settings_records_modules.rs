//! S7-11: modules: address via the observer, manifest check, enabling, configuring, removal.

mod settings_common;

use controllers::controllers::settings::SettingsController;
use controllers::i18n::tr;
use controllers::module_manifest;
use controllers::settings_types::{Action, Cmd, Op, Reply, SaveError};
use controllers::settings_view::SettingsMsg;
use controllers::{Controller, SettingsPage};
use serde_json::json;
use settings_common::*;

#[test]
fn settings_module_custom_url_requires_observer_check() {
    let mut c = opened(SettingsPage::Modules);
    for bad in [
        "http://modules.example.test/a.json",
        "ftp://x",
        "javascript:alert(1)",
        "https://a b",
        "",
    ] {
        edit(&mut c, "m:url", bad);
        let fx = send(&mut c, SettingsMsg::Act(Action::ModuleInstallUrl));
        assert!(
            cmds(&fx).is_empty(),
            "{bad:?}: scheme is not https: no request"
        );
    }
    edit(&mut c, "m:url", "https://modules.example.test/ask-me.json");
    let fx = send(&mut c, SettingsMsg::Act(Action::ModuleInstallUrl));
    assert_eq!(
        cmds(&fx),
        vec![Cmd::InstallModule(
            json!({"url": "https://modules.example.test/ask-me.json"})
        )],
        "observer check happens on the core side"
    );
    let installed =
        |c: &SettingsController| items(c).iter().map(|(t, _)| t.clone()).collect::<Vec<_>>();
    let before = installed(&c);
    // the core awaits a decision: the module is not installed and does not appear in the list
    send(&mut c, SettingsMsg::Reply(Reply::ModulePending));
    assert_eq!(
        c.view().banner.map(|b| b.text),
        Some(tr("set-module-pending"))
    );
    assert_eq!(
        installed(&c),
        before,
        "without the observer's decision the module is not installed"
    );
    // the core rejected it: error, the list is unchanged
    let err = SaveError {
        status: 422,
        code: "validation_failed".into(),
        fields: vec!["url".into()],
    };
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Module,
            error: err,
        }),
    );
    assert_eq!(installed(&c), before);
    assert_eq!(c.view().banner.map(|b| b.text), Some(tr("set-err-invalid")));
    // only a list from the core with installed = true adds a module
    let mut list = modules();
    list.push(json!({"id": "x", "name": "Extra", "installed": true, "enabled": true, "placement": "panel", "manifest": catalog_manifest("x"), "config": {}}));
    send(&mut c, SettingsMsg::Reply(Reply::Modules(list)));
    assert!(installed(&c).contains(&"Extra".to_string()));
}

#[test]
fn settings_module_manifest_validation() {
    let good = catalog_manifest("weather");
    assert!(module_manifest::validate(&good).is_empty());
    let mut no_id = good.clone();
    no_id.as_object_mut().map(|o| o.remove("id"));
    assert_eq!(
        module_manifest::validate(&no_id),
        vec!["id".to_string()],
        "no id"
    );
    let mut bad_place = good.clone();
    bad_place["placement"] = json!("window");
    assert_eq!(
        module_manifest::validate(&bad_place),
        vec!["placement".to_string()]
    );
    let mut extra = good;
    extra["unknown"] = json!(1);
    assert_eq!(
        module_manifest::validate(&extra),
        vec!["unknown".to_string()],
        "unknown field"
    );
    // in the market list a manifest that failed validation gets no "Install" button
    let c = opened(SettingsPage::Modules);
    let list = items(&c);
    let weather = list.iter().find(|(t, _)| t == "Weather").expect("weather");
    assert_eq!(weather.1, vec![Action::ModuleInstall("weather".into())]);
    let broken = list.iter().find(|(t, _)| t == "Broken").expect("broken");
    assert!(broken.1.is_empty(), "a broken manifest is not installed");
    assert!(shown_text(&c).contains(&tr("set-manifest-invalid")));
}

#[test]
fn settings_module_flow_toggle_config_remove() {
    let mut c = opened(SettingsPage::Modules);
    assert_eq!(
        cmds(&send(
            &mut c,
            SettingsMsg::Act(Action::ModuleInstall("weather".into()))
        )),
        vec![Cmd::InstallModule(json!({"catalog_id": "weather"}))]
    );
    send(&mut c, SettingsMsg::Reply(Reply::ModulePending));
    assert_eq!(
        cmds(&send(
            &mut c,
            SettingsMsg::Act(Action::ModuleToggle("notes".into()))
        )),
        vec![Cmd::PatchModule {
            id: "notes".into(),
            body: json!({"enabled": false})
        }]
    );
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Module)));
    assert_eq!(
        field(&c, "m:notes:cfg:city").map(|f| f.value),
        Some("Haifa".into()),
        "the manifest question became a field"
    );
    edit(&mut c, "m:notes:cfg:city", "Tel Aviv");
    edit(&mut c, "m:notes:placement", "widget");
    let sent = cmds(&send(&mut c, SettingsMsg::Save));
    assert_eq!(
        sent,
        vec![Cmd::PatchModule {
            id: "notes".into(),
            body: json!({"config": {"city": "Tel Aviv"}, "placement": "widget"})
        }]
    );
    send(
        &mut c,
        SettingsMsg::Act(Action::ModuleRemove("notes".into())),
    );
    assert!(c.view().confirm.is_some());
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Confirm(true))),
        vec![Cmd::DeleteModule("notes".into())]
    );
}

#[test]
fn settings_module_field_drafts_are_cleared_after_save() {
    let mut c = opened(SettingsPage::Modules);
    edit(&mut c, "m:notes:cfg:city", "Eilat");
    assert!(c.view().dirty);
    send(&mut c, SettingsMsg::Save);
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Module)));
    assert!(
        !c.view().dirty,
        "the saved module answer is no longer a draft"
    );
}
