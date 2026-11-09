//! Personas and permissions.

mod settings_common;

use controllers::i18n::tr;
use controllers::permissions::Mode;
use controllers::settings_types::{Action, Cmd, Op, Reply};
use controllers::settings_view::{Kind, SettingsMsg};
use controllers::{Controller, SettingsPage};
use serde_json::json;
use settings_common::*;

#[test]
fn settings_persona_rename_keeps_priority() {
    let mut c = opened(SettingsPage::Personas);
    let prio = field(&c, "d:personas:balance.priority_persona_id").expect("priority");
    assert_eq!(
        prio.value, "1",
        "the priority stores the persona number, not the name"
    );
    edit(&mut c, "p:1:name", "Boss");
    let fx = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&fx),
        vec![Cmd::PatchPersona {
            id: 1,
            body: json!({"name": "Boss"})
        }],
        "only the name is sent; the balance page is untouched"
    );
    // the core replies: persona renamed, balance page unchanged
    let mut renamed = personas();
    renamed[0]["name"] = json!("Boss");
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Persona)));
    send(&mut c, SettingsMsg::Reply(Reply::Personas(renamed)));
    let prio = field(&c, "d:personas:balance.priority_persona_id").expect("priority");
    assert_eq!(prio.value, "1", "the priority is the same after the rename");
    match prio.kind {
        Kind::Choice(options) => assert!(
            options.contains(&("1".to_string(), "Boss".to_string())),
            "{options:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn settings_persona_lang_en_selected() {
    let mut c = opened(SettingsPage::Personas);
    send(&mut c, SettingsMsg::Act(Action::PersonaSelect(2)));
    let lang = field(&c, "p:2:lang").expect("persona language");
    assert_eq!(lang.value, "en", "English is selected and shown");
    match lang.kind {
        Kind::Choice(options) => {
            let values: Vec<_> = options.iter().map(|(v, _)| v.as_str()).collect();
            assert_eq!(values, ["en"], "the GUI offers English only");
        }
        other => panic!("{other:?}"),
    }
    edit(&mut c, "p:2:lang", "en");
    assert!(
        !c.view().dirty,
        "returning to the saved value clears the draft"
    );
    // a persona saved with the protocol value `ru` is shown as is, without a draft
    send(&mut c, SettingsMsg::Act(Action::PersonaSelect(1)));
    let lang = field(&c, "p:1:lang").expect("persona language");
    assert_eq!(lang.value, "ru", "protocol value kept as data");
    assert!(!c.view().dirty);
}

#[test]
fn settings_persona_delete_requires_confirm() {
    let mut c = opened(SettingsPage::Personas);
    let fx = send(&mut c, SettingsMsg::Act(Action::PersonaDelete(1)));
    assert!(
        cmds(&fx).is_empty(),
        "nothing is deleted without confirmation"
    );
    let confirm = c.view().confirm.expect("confirmation");
    let text = confirm.lines.join("\n");
    assert!(text.contains("Work"), "{text}");
    assert!(
        text.contains(&tr("set-confirm-persona-refs")),
        "list: references in folders and sessions"
    );
    assert!(
        text.contains(&tr("set-confirm-persona-priority")),
        "the persona is the balance priority"
    );
    assert!(cmds(&send(&mut c, SettingsMsg::Confirm(false))).is_empty());
    assert!(c.view().confirm.is_none());
    send(&mut c, SettingsMsg::Act(Action::PersonaDelete(1)));
    let fx = send(&mut c, SettingsMsg::Confirm(true));
    assert_eq!(cmds(&fx), vec![Cmd::DeletePersona(1)]);
    // persona 2 is not the priority: no priority line in the confirmation
    send(&mut c, SettingsMsg::Act(Action::PersonaDelete(2)));
    let text = c
        .view()
        .confirm
        .map(|c| c.lines.join("\n"))
        .unwrap_or_default();
    assert!(!text.contains(&tr("set-confirm-persona-priority")));
    // after deletion the core reset the priority, the dialog rereads everything
    send(&mut c, SettingsMsg::Confirm(false));
    let fx = send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Persona)));
    assert_eq!(cmds(&fx), vec![Cmd::FetchAll]);
}

#[test]
fn settings_persona_create_requires_name_and_sends_accounts() {
    let mut c = opened(SettingsPage::Personas);
    send(&mut c, SettingsMsg::Act(Action::PersonaFormNew));
    let fx = send(&mut c, SettingsMsg::Act(Action::PersonaNew));
    assert!(cmds(&fx).is_empty(), "not created without a name");
    assert!(field(&c, "p:new:name").and_then(|f| f.error).is_some());
    edit(&mut c, "p:new:name", "Night");
    edit(&mut c, "p:new:lang", "en");
    edit(&mut c, "p:new:acc:2", "true");
    let fx = send(&mut c, SettingsMsg::Act(Action::PersonaNew));
    let sent = cmds(&fx);
    assert_eq!(sent.len(), 1);
    match &sent[0] {
        Cmd::CreatePersona(body) => {
            assert_eq!(body["name"], "Night");
            assert_eq!(body["lang"], "en");
            assert_eq!(
                body["color"], "#808080",
                "default colour: the core requires a colour"
            );
            assert_eq!(body["account_ids"], json!([2]));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn settings_permissions_cannot_loosen() {
    let mut c = opened(SettingsPage::Permissions);
    // globally `shell` = deny: a persona cannot set allow or ask
    let f = field(&c, "perm:persona:shell").expect("persona level");
    match &f.kind {
        Kind::Choice(options) => {
            let values: Vec<_> = options.iter().map(|(v, _)| v.as_str()).collect();
            assert_eq!(values, ["deny"], "no weakening values in the list");
        }
        other => panic!("{other:?}"),
    }
    edit(&mut c, "perm:persona:shell", "allow");
    let f = field(&c, "perm:persona:shell").expect("field");
    assert_eq!(
        f.error,
        Some(tr("set-err-loosen")),
        "manual bypass of the list is rejected with a reason"
    );
    assert!(
        !f.dirty && !c.view().dirty,
        "the weakening did not reach the draft"
    );
    assert!(cmds(&send(&mut c, SettingsMsg::Save)).is_empty());
    // default `send` = ask: a persona cannot go below it either
    edit(&mut c, "perm:persona:send", "allow");
    assert!(field(&c, "perm:persona:send").is_some_and(|f| f.error.is_some()));
    // tightening is accepted and goes to the core
    edit(&mut c, "perm:persona:send", "deny");
    let fx = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&fx),
        vec![Cmd::PutPermission {
            scope: "persona".into(),
            scope_id: 1,
            action: "send".into(),
            mode: Mode::Deny
        }]
    );
}

#[test]
fn settings_permissions_global_edit_and_ttl_saved() {
    let mut c = opened(SettingsPage::Permissions);
    edit(&mut c, "perm:global:read", "ask");
    edit(&mut c, "d:permissions:ask_ttl_min", "30");
    let sent = cmds(&send(&mut c, SettingsMsg::Save));
    assert!(
        sent.contains(&Cmd::PutPermission {
            scope: "global".into(),
            scope_id: 0,
            action: "read".into(),
            mode: Mode::Ask
        }),
        "{sent:?}"
    );
    assert!(
        sent.contains(&Cmd::PatchPage {
            doc: "permissions".into(),
            rev: 1,
            patch: json!({"ask_ttl_min": 30})
        }),
        "{sent:?}"
    );
    edit(&mut c, "d:permissions:ask_ttl_min", "0");
    assert!(
        field(&c, "d:permissions:ask_ttl_min").is_some_and(|f| f.error.is_some()),
        "TTL 0 is not accepted"
    );
}
