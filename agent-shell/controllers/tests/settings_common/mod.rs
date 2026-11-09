//! Shared helpers for Settings tests: a controller loaded with a synthetic core fixture.
#![allow(dead_code)]

use std::time::Instant;

use controllers::controllers::settings::SettingsController;
use controllers::settings_types::{Action, Cmd, FileEntry, Reply};
use controllers::settings_view::{FieldRow, Row, SettingsMsg, SettingsView};
use controllers::{Controller, CoreCommand, Effect, SettingsPage};
use serde_json::{Value, json};

pub fn send(c: &mut SettingsController, msg: SettingsMsg) -> Vec<Effect> {
    c.update(msg, Instant::now())
}

pub fn cmds(fx: &[Effect]) -> Vec<Cmd> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Command(CoreCommand::Settings(c)) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

pub fn page_reply(doc: &str, value: Value, rev: i64) -> Reply {
    Reply::Page {
        doc: doc.into(),
        value,
        rev,
        saved: false,
    }
}

pub fn saved_reply(doc: &str, value: Value, rev: i64) -> Reply {
    Reply::Page {
        doc: doc.into(),
        value,
        rev,
        saved: true,
    }
}

pub fn personas() -> Vec<Value> {
    vec![
        json!({"id": 1, "name": "Work", "color": "#112233", "lang": "ru", "mem_mode": "own", "notif_mode": "all", "account_ids": [2]}),
        json!({"id": 2, "name": "Home", "color": "#445566", "lang": "en", "mem_mode": "shared", "notif_mode": "digest", "account_ids": []}),
    ]
}

pub fn accounts() -> Vec<Value> {
    vec![
        json!({"id": 1, "type": "anthropic", "label": "Claude", "status": "connected"}),
        json!({"id": 2, "type": "tg", "label": "Telegram", "status": "connected"}),
        json!({"id": 3, "type": "wa", "label": "WhatsApp", "status": "disconnected"}),
        json!({"id": 4, "type": "ollama", "label": "Local", "status": "connected"}),
    ]
}

pub fn catalog_manifest(id: &str) -> Value {
    json!({"id": id, "name": id, "version": "1.0", "entry": "index.html", "placement": "widget"})
}

pub fn modules() -> Vec<Value> {
    vec![
        json!({"id": "notes", "name": "Notes", "installed": true, "enabled": true, "placement": "panel",
               "manifest": {"id": "notes", "name": "Notes", "version": "1", "entry": "i.html", "placement": "panel", "questions": ["city"]},
               "config": {"city": "Haifa"}}),
        json!({"id": "weather", "name": "Weather", "installed": false, "enabled": false, "placement": "widget",
               "manifest": catalog_manifest("weather"), "config": {}}),
        json!({"id": "broken", "name": "Broken", "installed": false, "enabled": false, "placement": "widget",
               "manifest": {"id": "broken", "name": "B", "version": "1", "entry": "../x", "placement": "window"}, "config": {}}),
    ]
}

pub fn perms(rows: &[(&str, i64, &str, &str)]) -> Value {
    Value::Array(
        rows.iter()
            .map(|(scope, id, action, mode)| json!({"scope": scope, "scope_id": id, "action": action, "mode": mode}))
            .collect(),
    )
}

/// All core responses that fill an open dialog.
pub fn replies() -> Vec<Reply> {
    vec![
        page_reply("account", json!({"name": "Alex", "language": "ru"}), 3),
        page_reply(
            "about",
            json!({"core_version": "0.1.0", "api": {"major": 1, "minor": 2}, "db_version": 7, "capabilities": ["a", "b"]}),
            1,
        ),
        page_reply(
            "personas",
            json!({"balance": {"fallback": "local", "priority_persona_id": 1}, "roles": {}}),
            5,
        ),
        page_reply(
            "models",
            json!({"accounts": {"1": {"model": "claude", "secret_ref": "account.1"}}, "context_budget": 8000}),
            2,
        ),
        page_reply("connections", json!({"mcp": []}), 1),
        page_reply("permissions", json!({"ask_ttl_min": 10}), 1),
        page_reply(
            "security",
            json!({"ptrace_scope": 1, "disk_encryption": "unknown", "secrets_present": true}),
            1,
        ),
        page_reply(
            "schedule",
            json!({"summary": {"enabled": true, "time": "08:00"}, "backup": {"enabled": false}}),
            4,
        ),
        page_reply("memory", json!({"source": "local"}), 1),
        page_reply("tunnels", json!({"api_external": false}), 6),
        page_reply(
            "indicators",
            json!({"items": [{"id": "cpu", "enabled": true, "position": 0}, {"id": "ram", "enabled": true, "position": 1}]}),
            2,
        ),
        Reply::Personas(personas()),
        Reply::Accounts(accounts()),
        Reply::Modules(modules()),
        Reply::Tunnels(vec![
            json!({"id": 1, "type": "ssh", "addr": "h:22", "descr": "d", "enabled": false, "status": "down"}),
        ]),
        Reply::Files(vec![
            FileEntry {
                path: "AGENT.md".into(),
                size: 10,
                rev: "a".into(),
            },
            FileEntry {
                path: "memory/notes.md".into(),
                size: 5,
                rev: "b".into(),
            },
            FileEntry {
                path: "keys.env".into(),
                size: 3,
                rev: "c".into(),
            },
            FileEntry {
                path: "permissions.json".into(),
                size: 3,
                rev: "d".into(),
            },
        ]),
        Reply::Secrets(vec![]),
        Reply::Perms(perms(&[
            ("global", 0, "send", "ask"),
            ("global", 0, "shell", "deny"),
        ])),
    ]
}

/// A dialog open on `page` with its data loaded.
pub fn opened(page: SettingsPage) -> SettingsController {
    let mut c = SettingsController::new();
    send(&mut c, SettingsMsg::Open(page));
    for r in replies() {
        send(&mut c, SettingsMsg::Reply(r));
    }
    c
}

pub fn rows(c: &SettingsController) -> Vec<Row> {
    c.view().rows
}

pub fn field(c: &SettingsController, id: &str) -> Option<FieldRow> {
    rows(c).into_iter().find_map(|r| match r {
        Row::Field(f) if f.id == id => Some(f),
        _ => None,
    })
}

pub fn edit(c: &mut SettingsController, id: &str, value: &str) {
    send(
        c,
        SettingsMsg::Edit {
            id: id.into(),
            value: value.into(),
        },
    );
}

pub fn view(c: &SettingsController) -> SettingsView {
    c.view()
}

/// All text shown by the view as one string (for searching forbidden words).
pub fn shown_text(c: &SettingsController) -> String {
    let v = c.view();
    let mut out = vec![v.title.clone()];
    for g in &v.nav {
        out.push(g.title.clone());
        out.extend(g.items.iter().map(|i| i.label.clone()));
    }
    for r in &v.rows {
        match r {
            Row::Heading(t) | Row::Note(t) => out.push(t.clone()),
            Row::Field(f) => {
                out.push(f.label.clone());
                out.push(f.value.clone());
            }
            Row::Item(i) => {
                out.push(i.title.clone());
                out.push(i.subtitle.clone());
                out.extend(i.tags.clone());
                out.extend(i.actions.iter().map(|a| a.label.clone()));
            }
            Row::Button(a) => out.push(a.label.clone()),
        }
    }
    out.join("\n")
}

/// Entries of a list page: the heading and the actions.
pub fn items(c: &SettingsController) -> Vec<(String, Vec<Action>)> {
    rows(c)
        .into_iter()
        .filter_map(|r| match r {
            Row::Item(i) => Some((i.title, i.actions.into_iter().map(|a| a.action).collect())),
            _ => None,
        })
        .collect()
}
