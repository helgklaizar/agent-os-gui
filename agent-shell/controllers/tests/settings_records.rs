//! S7-06, S7-08, S7-12: tunnels and connections (accounts, MCP), new-record forms.

mod settings_common;

use controllers::i18n::tr;
use controllers::settings_types::{Action, Cmd, Op, Reply, SaveError};
use controllers::settings_view::{Row, SettingsMsg};
use controllers::{Controller, SettingsPage};
use serde_json::json;
use settings_common::*;

#[test]
fn settings_tunnel_add_toggle_delete_with_confirm() {
    let mut c = opened(SettingsPage::Tunnels);
    assert!(
        cmds(&send(&mut c, SettingsMsg::Act(Action::TunnelAdd))).is_empty(),
        "address is required"
    );
    edit(&mut c, "t:new:type", "wireguard");
    edit(&mut c, "t:new:addr", "10.0.0.1:51820");
    edit(&mut c, "t:new:descr", "<b>home</b>");
    let sent = cmds(&send(&mut c, SettingsMsg::Act(Action::TunnelAdd)));
    assert_eq!(
        sent,
        vec![Cmd::CreateTunnel(
            json!({"type": "wireguard", "addr": "10.0.0.1:51820", "descr": "<b>home</b>"})
        )]
    );
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Tunnel)));
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Act(Action::TunnelToggle(1)))),
        vec![Cmd::PatchTunnel {
            id: 1,
            body: json!({"enabled": true})
        }]
    );
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Tunnel)));
    assert!(cmds(&send(&mut c, SettingsMsg::Act(Action::TunnelDelete(1)))).is_empty());
    assert!(
        c.view().confirm.is_some(),
        "tunnel deletion goes through confirmation"
    );
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Confirm(true))),
        vec![Cmd::DeleteTunnel(1)]
    );
    assert!(
        cmds(&send(&mut c, SettingsMsg::Save)).is_empty(),
        "while there is no reply to the command, a second save does not start"
    );
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Tunnel)));
    edit(&mut c, "d:tunnels:api_external", "true");
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Save)),
        vec![Cmd::PatchPage {
            doc: "tunnels".into(),
            rev: 6,
            patch: json!({"api_external": true})
        }]
    );
}

#[test]
fn connections_unofficial_channel_label() {
    let c = opened(SettingsPage::Connections);
    let tags = |title: &str| {
        rows(&c).into_iter().find_map(|r| match r {
            Row::Item(i) if i.title == title => Some(i.tags),
            _ => None,
        })
    };
    assert_eq!(
        tags("WhatsApp"),
        Some(vec![tr("set-unofficial")]),
        "Whapi/WhatsApp is marked as an unofficial channel (Q9)"
    );
    assert_eq!(tags("Telegram"), Some(vec![]));
}

#[test]
fn settings_connections_add_account_and_mcp() {
    let mut c = opened(SettingsPage::Connections);
    assert!(
        cmds(&send(&mut c, SettingsMsg::Act(Action::AccountAdd))).is_empty(),
        "label is required"
    );
    edit(&mut c, "a:new:label", "Work TG");
    edit(&mut c, "a:new:type", "tg");
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Act(Action::AccountAdd))),
        vec![Cmd::CreateAccount(
            json!({"type": "tg", "label": "Work TG"})
        )]
    );
    assert_eq!(
        cmds(&send(&mut c, SettingsMsg::Act(Action::AccountConnect(3)))),
        vec![Cmd::ConnectAccount(3)]
    );
    edit(&mut c, "c:new:name", "Bad Name");
    edit(&mut c, "c:new:url", "ftp://x");
    assert!(cmds(&send(&mut c, SettingsMsg::Act(Action::McpAdd))).is_empty());
    assert!(
        field(&c, "c:new:name").is_some_and(|f| f.error.is_some())
            && field(&c, "c:new:url").is_some_and(|f| f.error.is_some())
    );
    edit(&mut c, "c:new:name", "tools");
    edit(&mut c, "c:new:url", "https://mcp.example.test/sse");
    let sent = cmds(&send(&mut c, SettingsMsg::Act(Action::McpAdd)));
    assert_eq!(
        sent,
        vec![Cmd::PatchPage {
            doc: "connections".into(),
            rev: 1,
            patch: json!({"mcp": [{"name": "tools", "transport": "http", "url": "https://mcp.example.test/sse", "enabled": true}]})
        }]
    );
    send(&mut c, SettingsMsg::Act(Action::AccountDelete(3)));
    assert!(
        c.view().confirm.is_some(),
        "account deletion requires confirmation"
    );
}

#[test]
fn settings_no_key_rotation_in_ui() {
    for page in [
        SettingsPage::Personas,
        SettingsPage::Models,
        SettingsPage::Connections,
        SettingsPage::Security,
    ] {
        let c = opened(page);
        let text = shown_text(&c).to_lowercase();
        assert!(!text.contains("rotat"), "{page:?}: no key rotation in 0.1");
        for r in rows(&c) {
            if let Row::Field(f) = r {
                assert!(!f.id.contains("rotate"), "{}", f.id);
            }
        }
    }
}

#[test]
fn settings_new_record_forms_keep_input_on_failure_and_clear_on_success() {
    let mut c = opened(SettingsPage::Tunnels);
    edit(&mut c, "t:new:addr", "10.0.0.9:22");
    send(&mut c, SettingsMsg::Act(Action::TunnelAdd));
    let err = SaveError {
        status: 500,
        code: "internal".into(),
        fields: vec![],
    };
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Tunnel,
            error: err,
        }),
    );
    assert_eq!(
        field(&c, "t:new:addr").map(|f| f.value),
        Some("10.0.0.9:22".into()),
        "input is kept after a failure"
    );
    send(&mut c, SettingsMsg::Act(Action::TunnelAdd));
    send(&mut c, SettingsMsg::Reply(Reply::Done(Op::Tunnel)));
    assert_eq!(
        field(&c, "t:new:addr").map(|f| f.value),
        Some(String::new()),
        "the form is empty after success"
    );
    assert!(!c.view().dirty);
}
