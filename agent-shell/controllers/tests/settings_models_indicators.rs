//! S7-06 … S7-13: models, indicators, consistency across pages.

mod settings_common;

use controllers::controllers::settings::SettingsController;
use controllers::settings_types::{Action, Cmd, Reply};
use controllers::settings_view::{Row, SettingsMsg};
use controllers::{Controller, SettingsPage};
use serde_json::{Value, json};
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
fn settings_indicators_order_and_enable() {
    let mut c = opened(SettingsPage::Indicators);
    let order = |c: &SettingsController| items(c).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(order(&c), ["cpu", "ram"]);
    send(
        &mut c,
        SettingsMsg::Act(Action::IndicatorDown("cpu".into())),
    );
    send(
        &mut c,
        SettingsMsg::Act(Action::IndicatorToggle("ram".into())),
    );
    assert_eq!(order(&c), ["ram", "cpu"]);
    assert!(c.view().dirty);
    let sent = cmds(&send(&mut c, SettingsMsg::Save));
    assert_eq!(
        sent,
        vec![Cmd::PatchPage {
            doc: "indicators".into(),
            rev: 2,
            patch: json!({"items": [
                {"id": "ram", "enabled": false, "position": 0},
                {"id": "cpu", "enabled": true, "position": 1}
            ]})
        }]
    );
    let saved = json!({"items": [{"id": "ram", "enabled": false, "position": 0}, {"id": "cpu", "enabled": true, "position": 1}]});
    send(
        &mut c,
        SettingsMsg::Reply(saved_reply("indicators", saved, 3)),
    );
    assert!(!c.view().dirty);
    assert_eq!(
        order(&c),
        ["ram", "cpu"],
        "saved order is shown from the core reply"
    );
}

#[test]
fn settings_indicator_edges_have_no_dead_moves() {
    let c = opened(SettingsPage::Indicators);
    let list = items(&c);
    assert!(
        !list[0]
            .1
            .iter()
            .any(|a| matches!(a, Action::IndicatorUp(_))),
        "the first one cannot move up"
    );
    assert!(
        !list[1]
            .1
            .iter()
            .any(|a| matches!(a, Action::IndicatorDown(_))),
        "the last one cannot move down"
    );
}

#[test]
fn settings_pages_consistent_on_one_fixture_controller_level() {
    // "Connections", "Models", "Personas" read one model: an account looks the same on all three
    let mut c = opened(SettingsPage::Connections);
    let seen = |c: &SettingsController, needle: &str| shown_text(c).contains(needle);
    assert!(seen(&c, "Claude") && seen(&c, "Telegram"));
    send(&mut c, SettingsMsg::Select(SettingsPage::Models));
    assert!(
        seen(&c, "Claude") && !seen(&c, "Telegram"),
        "Models shows only AI accounts"
    );
    send(&mut c, SettingsMsg::Select(SettingsPage::Personas));
    assert!(seen(&c, "Telegram"), "persona accounts are the same");
    // a new account from the core appears everywhere without a restart
    let mut list = accounts();
    list.push(json!({"id": 9, "type": "gmail", "label": "Mail9", "status": "connected"}));
    send(&mut c, SettingsMsg::Reply(Reply::Accounts(list)));
    assert!(seen(&c, "Mail9"));
    send(&mut c, SettingsMsg::Select(SettingsPage::Connections));
    assert!(seen(&c, "Mail9"));
}

#[test]
fn settings_models_local_state_matches_bar_quota() {
    let mut c = opened(SettingsPage::Models);
    let q = controllers::bar_types::parse_quota_snapshot(&json!({"items": [
        {"account_id": 4, "label": "Local", "kind": "local", "load": 0.42},
        {"account_id": 1, "label": "Claude", "kind": "api_key", "used": 0.5}
    ]}))
    .expect("snapshot");
    send(&mut c, SettingsMsg::Quotas(q));
    let local = shown_text(&c);
    assert!(
        local.contains("42%"),
        "local model load matches the value in the bar snapshot: {local}"
    );
    let _: Value = json!(null);
}
