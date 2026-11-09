//! S7-06 … S7-13: secrets and files.

mod settings_common;

use controllers::controllers::settings::SettingsController;
use controllers::i18n::tr;
use controllers::settings_types::{Action, Cmd, Op, Reply, SaveError, Secret};
use controllers::settings_view::{Kind, Row, SettingsMsg};
use controllers::{Controller, SettingsPage};
use serde_json::json;
use settings_common::*;

fn items(c: &SettingsController) -> Vec<(String, Vec<Action>)> {
    rows(c)
        .into_iter()
        .filter_map(|r| match r {
            Row::Item(i) => Some((i.title, i.actions.into_iter().map(|a| a.action).collect())),
            _ => None,
        })
        .collect()
}

#[test]
fn settings_files_editor_only_agentmd_and_memory() {
    let mut c = opened(SettingsPage::Files);
    let list = items(&c);
    let titles: Vec<_> = list.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        ["AGENT.md", "memory/notes.md"],
        "keys.env and permissions.json are not in the list even when the core replies with them"
    );
    for bad in [
        "keys.env",
        "permissions.json",
        "memory/../keys.env",
        "/etc/passwd",
        "personas.json",
        "~/.agentos/keys.env",
    ] {
        let fx = send(&mut c, SettingsMsg::Act(Action::FileOpen(bad.into())));
        assert!(cmds(&fx).is_empty(), "{bad}: no request to the core");
        assert_eq!(
            c.view().banner.map(|b| b.text),
            Some(tr("set-err-path")),
            "{bad}"
        );
    }
    let text = shown_text(&c).to_lowercase();
    assert!(
        !text.contains("terminal") && !text.contains("~/.agentos") && !text.contains("keys.env"),
        "{text}"
    );
    let fx = send(
        &mut c,
        SettingsMsg::Act(Action::FileOpen("AGENT.md".into())),
    );
    assert_eq!(cmds(&fx), vec![Cmd::FetchFile("AGENT.md".into())]);
    send(
        &mut c,
        SettingsMsg::Reply(Reply::File {
            path: "AGENT.md".into(),
            text: "hello".into(),
            rev: "r1".into(),
        }),
    );
    assert_eq!(field(&c, "f:text").map(|f| f.value), Some("hello".into()));
    edit(&mut c, "f:text", "hello world");
    let fx = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&fx),
        vec![Cmd::PutFile {
            path: "AGENT.md".into(),
            rev: "r1".into(),
            text: "hello world".into()
        }]
    );
    send(
        &mut c,
        SettingsMsg::Reply(Reply::FileSaved {
            path: "AGENT.md".into(),
            rev: "r2".into(),
        }),
    );
    assert!(!c.view().dirty);
    assert_eq!(
        field(&c, "f:text").map(|f| f.value),
        Some("hello world".into())
    );
}

#[test]
fn settings_secret_fields_write_only() {
    let mut c = opened(SettingsPage::Models);
    let key = field(&c, "s:account.1").expect("key field");
    assert_eq!(key.kind, Kind::Secret);
    assert_eq!(key.value, "", "the key is not shown");
    send(
        &mut c,
        SettingsMsg::EditSecret {
            id: "s:account.1".into(),
            value: Secret::new("sk-live-TOPSECRET"),
        },
    );
    assert!(
        field(&c, "s:account.1").is_some_and(|f| f.dirty && f.value.is_empty()),
        "typed input is not reflected in the view"
    );
    assert!(!format!("{:?}", c.view()).contains("TOPSECRET"));
    let fx = send(&mut c, SettingsMsg::Save);
    assert!(
        !format!("{fx:?}").contains("TOPSECRET"),
        "the value is absent from effects Debug output (logs)"
    );
    let sent = cmds(&fx);
    assert!(
        matches!(&sent[0], Cmd::PutSecret { reference, value } if reference == "account.1" && value.expose() == "sk-live-TOPSECRET"),
        "the secret goes to the core"
    );
    assert!(sent.iter().any(|c| matches!(c, Cmd::PatchPage { doc, patch, .. } if doc == "models" && patch["accounts"]["1"]["secret_ref"] == "account.1")), "only the reference goes into the document");
    assert!(
        !sent.iter().any(
            |c| matches!(c, Cmd::PatchPage { patch, .. } if patch.to_string().contains("TOPSECRET"))
        ),
        "the value is not in the document"
    );
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Done(Op::Secret("account.1".into()))),
    );
    send(
        &mut c,
        SettingsMsg::Reply(saved_reply(
            "models",
            json!({"accounts": {"1": {"model": "claude", "secret_ref": "account.1"}}, "context_budget": 8000}),
            3,
        )),
    );
    let key = field(&c, "s:account.1").expect("field");
    assert_eq!(key.value, "", "the field is empty after saving");
    assert!(!key.dirty && !c.view().dirty);
    assert!(
        shown_text(&c).contains(&tr("set-key-set")),
        "only \"key is set\" is shown"
    );
}

#[test]
fn settings_secret_failure_keeps_input_hidden() {
    let mut c = opened(SettingsPage::Models);
    send(
        &mut c,
        SettingsMsg::EditSecret {
            id: "s:account.1".into(),
            value: Secret::new("k1"),
        },
    );
    send(&mut c, SettingsMsg::Save);
    let err = SaveError {
        status: 403,
        code: "permission_denied".into(),
        fields: vec![],
    };
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Secret("account.1".into()),
            error: err,
        }),
    );
    assert!(
        field(&c, "s:account.1")
            .is_some_and(|f| f.dirty && f.error.is_some() && f.value.is_empty())
    );
    send(
        &mut c,
        SettingsMsg::EditSecret {
            id: "s:account.1".into(),
            value: Secret::new(""),
        },
    );
    assert!(!c.view().dirty, "empty input cancels the secret");
}
