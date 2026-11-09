//! Module manifest validation boundaries and small list actions.

mod settings_common;

use controllers::SettingsPage;
use controllers::module_manifest::validate;
use controllers::settings_types::{Action, Reply};
use controllers::settings_view::{Row, SettingsMsg};
use serde_json::{Value, json};
use settings_common::*;

fn manifest(patch: Value) -> Value {
    let mut m = json!({"id": "a", "name": "N", "version": "1", "entry": "index.html", "placement": "panel"});
    if let (Value::Object(a), Value::Object(b)) = (&mut m, patch) {
        a.extend(b);
    }
    m
}

fn bad(patch: Value) -> Vec<String> {
    validate(&manifest(patch))
}

#[test]
fn manifest_id_boundaries() {
    assert!(bad(json!({"id": "a"})).is_empty());
    assert!(
        bad(json!({"id": "9a-b"})).is_empty(),
        "a leading digit is allowed"
    );
    assert!(bad(json!({"id": "a".repeat(40)})).is_empty());
    assert_eq!(bad(json!({"id": "a".repeat(41)})), ["id"]);
    for id in ["", "-a", "A", "a_b", "a b", "é"] {
        assert_eq!(bad(json!({"id": id})), ["id"], "{id:?}");
    }
    assert_eq!(bad(json!({"id": 5})), ["id"]);
}

#[test]
fn manifest_text_fields_boundaries() {
    assert!(bad(json!({"name": "n".repeat(80)})).is_empty());
    assert_eq!(bad(json!({"name": "n".repeat(81)})), ["name"]);
    assert_eq!(
        bad(json!({"name": "   "})),
        ["name"],
        "spaces count as empty"
    );
    assert!(bad(json!({"version": "v".repeat(32)})).is_empty());
    assert_eq!(bad(json!({"version": "v".repeat(33)})), ["version"]);
    assert_eq!(bad(json!({"version": ""})), ["version"]);
}

#[test]
fn manifest_entry_boundaries() {
    let long_ok = format!("{}.html", "e".repeat(115));
    assert_eq!(long_ok.len(), 120);
    assert!(bad(json!({"entry": long_ok})).is_empty());
    assert_eq!(
        bad(json!({"entry": format!("{}x", "e".repeat(120))})),
        ["entry"]
    );
    for e in [
        "", "/a", "a\\b", "a:b", "a//b", "./a", "a/./b", "a/../b", "a/", "a\u{0}b",
    ] {
        assert_eq!(bad(json!({"entry": e})), ["entry"], "{e:?}");
    }
    assert!(bad(json!({"entry": "dir/sub/page.html"})).is_empty());
}

#[test]
fn manifest_permissions_rules() {
    assert!(
        bad(json!({"permissions": ["GET /sessions", "POST /sessions/*", "DELETE /a_b-c"]}))
            .is_empty()
    );
    let path120 = format!("GET /{}", "p".repeat(119));
    assert!(bad(json!({"permissions": [path120]})).is_empty());
    assert_eq!(
        bad(json!({"permissions": [format!("GET /{}", "p".repeat(120))]})),
        ["permissions[0]"]
    );
    for p in [
        "get /x",
        "FETCH /x",
        "GET x",
        "GET",
        "GET /a.b",
        "GET /a b",
        "GET /tokens",
        "GET /tokens/1",
        "PUT /secrets*",
        "GET /settings/theme",
        "POST /modules",
        "GET /logout",
    ] {
        assert_eq!(bad(json!({"permissions": [p]})), ["permissions[0]"], "{p}");
    }
    assert!(
        bad(json!({"permissions": ["GET /tokensx"]})).is_empty(),
        "another scope, not tokens"
    );
    assert_eq!(
        bad(json!({"permissions": ["GET /x", 5]})),
        ["permissions[1]"]
    );
    let many: Vec<String> = (0..32).map(|i| format!("GET /p{i}")).collect();
    assert!(bad(json!({"permissions": many})).is_empty());
    let too_many: Vec<String> = (0..33).map(|i| format!("GET /p{i}")).collect();
    assert_eq!(bad(json!({"permissions": too_many})), ["permissions"]);
    assert_eq!(bad(json!({"permissions": "GET /x"})), ["permissions"]);
}

#[test]
fn manifest_network_and_fs_rules() {
    assert!(bad(json!({"network": ["api.example.test", "a-b.c1.io"]})).is_empty());
    let d253 = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(d253.len(), 253);
    assert!(bad(json!({"network": [d253.clone()]})).is_empty());
    assert_eq!(
        bad(json!({"network": [format!("{d253}x")]})),
        ["network[0]"]
    );
    for d in [
        "localhost",
        ".a.b",
        "-a.b",
        "A.b",
        "a_b.c",
        "https://x.test",
        "a.b/c",
        "a b.c",
        "",
    ] {
        assert_eq!(bad(json!({"network": [d]})), ["network[0]"], "{d:?}");
    }
    assert!(bad(json!({"fs": ["~/docs", "~/a/b"]})).is_empty());
    for f in ["~/", "~", "/etc", "~/a/../b", "~/./a", "docs"] {
        assert_eq!(bad(json!({"fs": [f]})), ["fs[0]"], "{f:?}");
    }
    assert_eq!(bad(json!({"questions": {"a": 1}})), ["questions"]);
    assert!(bad(json!({"questions": []})).is_empty());
}

#[test]
fn module_toggle_enables_a_disabled_module() {
    let mut c = opened(SettingsPage::Modules);
    let mut list = modules();
    list[0]["enabled"] = json!(false);
    send(&mut c, SettingsMsg::Reply(Reply::Modules(list)));
    let fx = send(
        &mut c,
        SettingsMsg::Act(Action::ModuleToggle("notes".into())),
    );
    assert_eq!(
        cmds(&fx),
        vec![controllers::settings_types::Cmd::PatchModule {
            id: "notes".into(),
            body: json!({"enabled": true})
        }]
    );
}

#[test]
fn indicator_moves_at_the_edges_do_not_break_the_list() {
    let mut c = opened(SettingsPage::Indicators);
    let order = |c: &controllers::controllers::settings::SettingsController| {
        rows(c)
            .into_iter()
            .filter_map(|r| {
                if let Row::Item(i) = r {
                    Some(i.title)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    send(
        &mut c,
        SettingsMsg::Act(Action::IndicatorDown("ram".into())),
    );
    send(&mut c, SettingsMsg::Act(Action::IndicatorUp("cpu".into())));
    send(&mut c, SettingsMsg::Act(Action::IndicatorUp("nope".into())));
    assert_eq!(
        order(&c),
        ["cpu", "ram"],
        "edges and an unknown indicator do not change the order"
    );
    send(&mut c, SettingsMsg::Act(Action::IndicatorUp("ram".into())));
    assert_eq!(order(&c), ["ram", "cpu"]);
}
