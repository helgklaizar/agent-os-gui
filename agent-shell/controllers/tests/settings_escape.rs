//! Hostile strings in names, labels, addresses and paths reach the view literally and break nothing.
//! The escaping itself (`set_use_markup(false)`) is checked by the UI scenario `ui_settings_text_is_escaped`.

mod settings_common;

use controllers::SettingsPage;
use controllers::settings_types::{Action, Cmd, Reply};
use controllers::settings_view::{Row, SettingsMsg};
use controllers::testkit::evil_strings;
use serde_json::json;
use settings_common::*;

#[test]
fn settings_text_is_escaped() {
    for evil in evil_strings() {
        let mut c = opened(SettingsPage::Personas);
        let mut people = personas();
        people[0]["name"] = json!(evil);
        people[0]["signature"] = json!(evil);
        send(&mut c, SettingsMsg::Reply(Reply::Personas(people)));
        send(
            &mut c,
            SettingsMsg::Reply(Reply::Tunnels(vec![
                json!({"id": 1, "type": "ssh", "addr": evil, "descr": evil, "enabled": true, "status": "up"}),
            ])),
        );
        let mut mods = modules();
        mods[0]["name"] = json!(evil);
        send(&mut c, SettingsMsg::Reply(Reply::Modules(mods)));
        let personas_text = shown_text(&c);
        assert!(
            personas_text.contains(&evil),
            "the persona name is shown literally"
        );
        send(&mut c, SettingsMsg::Select(SettingsPage::Tunnels));
        let tunnels_text = shown_text(&c);
        assert!(
            tunnels_text.contains(&evil),
            "the tunnel address and description are shown literally"
        );
        send(&mut c, SettingsMsg::Select(SettingsPage::Modules));
        assert!(
            shown_text(&c).contains(&evil),
            "the module name is shown literally"
        );
    }
}

#[test]
fn settings_evil_input_is_bounded_not_rewritten() {
    for evil in evil_strings() {
        let mut c = opened(SettingsPage::Personas);
        send(&mut c, SettingsMsg::Act(Action::PersonaFormNew));
        edit(&mut c, "p:new:name", &evil);
        let fx = send(&mut c, SettingsMsg::Act(Action::PersonaNew));
        let sent = cmds(&fx);
        if evil.chars().count() <= 80 {
            match sent.as_slice() {
                [Cmd::CreatePersona(body)] => {
                    assert_eq!(body["name"], json!(evil), "sent as is, without edits")
                }
                other => panic!("{other:?}"),
            }
        } else {
            assert!(
                sent.is_empty(),
                "a long string (100k) is rejected, not silently truncated"
            );
            assert!(
                field(&c, "p:new:name").is_some_and(|f| f.error.is_some() && f.value == evil),
                "the input stayed"
            );
        }
    }
}

#[test]
fn settings_file_with_markup_in_name_is_shown_literally() {
    let mut c = opened(SettingsPage::Files);
    let files = vec![
        controllers::settings_types::FileEntry {
            path: "memory/<img src=x onerror=alert(1)>.md".into(),
            size: 1,
            rev: "x".into(),
        },
        controllers::settings_types::FileEntry {
            path: "memory/ok.md".into(),
            size: 1,
            rev: "y".into(),
        },
    ];
    send(&mut c, SettingsMsg::Reply(Reply::Files(files)));
    let titles: Vec<_> = rows(&c)
        .into_iter()
        .filter_map(|r| {
            if let Row::Item(i) = r {
                Some(i.title)
            } else {
                None
            }
        })
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("<img")),
        "a valid name with markup is shown literally: {titles:?}"
    );
    assert!(titles.contains(&"memory/ok.md".to_string()));
}
