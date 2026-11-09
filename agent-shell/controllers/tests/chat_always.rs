//! S4-07: "Always" picks the rule level (persona or global), with no session level (D2).

mod chat_common;

use chat_common::*;
use controllers::Controller;
use controllers::chat_view::{ButtonKind, CardState};
use controllers::controllers::chat::ChatMsg;
use controllers::permissions::{Mode, PermDecision, PermTable, RuleScope, RuleSpec};
use serde_json::json;

#[test]
fn ask_always_sends_rule_scope() {
    let mut c = open_with_card();
    assert!(
        commands(&click(&mut c, ButtonKind::Always)).is_empty(),
        "\"Always\" asks for the level first"
    );
    let rule = |scope, scope_id| RuleSpec {
        scope,
        scope_id,
        mode: Mode::Allow,
    };
    let fx = step(
        &mut c,
        ChatMsg::AlwaysPick {
            request_id: REQ,
            scope: RuleScope::Persona,
            mode: Mode::Allow,
        },
    );
    assert_eq!(
        commands(&fx),
        vec![&decide(
            PermDecision::Always,
            Some(rule(RuleScope::Persona, WORK))
        )]
    );
    assert_eq!(asks(&c.view())[0].state_label.as_deref(), Some("always"));

    let mut c = open_with_card();
    click(&mut c, ButtonKind::Always);
    let fx = step(
        &mut c,
        ChatMsg::AlwaysPick {
            request_id: REQ,
            scope: RuleScope::Global,
            mode: Mode::Ask,
        },
    );
    let want = RuleSpec {
        scope: RuleScope::Global,
        scope_id: 0,
        mode: Mode::Ask,
    };
    assert_eq!(
        commands(&fx),
        vec![&decide(PermDecision::Always, Some(want))]
    );
}

#[test]
fn ask_always_default_level_is_persona() {
    let mut c = open_with_card();
    click(&mut c, ButtonKind::Always);
    let v = c.view();
    let chooser = asks(&v)[0]
        .always
        .clone()
        .expect("the level chooser is open");
    let scopes: Vec<_> = chooser
        .levels
        .iter()
        .map(|l| (l.scope, l.scope_id, l.default_mode))
        .collect();
    assert_eq!(
        scopes,
        [
            (RuleScope::Persona, WORK, Mode::Allow),
            (RuleScope::Global, 0, Mode::Allow)
        ]
    );
}

#[test]
fn ask_always_no_session_level() {
    let mut c = open_with_card();
    let rows = json!([{ "scope": "global", "scope_id": 0, "action": "send", "mode": "deny" }]);
    step(&mut c, ChatMsg::Permissions(PermTable::from_value(&rows)));
    click(&mut c, ButtonKind::Always);
    let v = c.view();
    let chooser = asks(&v)[0].always.clone().unwrap();
    assert_eq!(
        chooser.levels.len(),
        2,
        "persona and global only, no session level"
    );
    assert_eq!(
        chooser.levels[0].modes,
        [Mode::Deny],
        "allow and ask are weaker than the global deny"
    );
    assert_eq!(chooser.levels[0].default_mode, Mode::Deny);
    let fx = step(
        &mut c,
        ChatMsg::AlwaysPick {
            request_id: REQ,
            scope: RuleScope::Persona,
            mode: Mode::Allow,
        },
    );
    assert!(
        commands(&fx).is_empty(),
        "a loosening value does not go to the core"
    );
    assert!(
        asks(&c.view())[0].always.is_some(),
        "the chooser stays open"
    );
}

#[test]
fn ask_always_without_persona_offers_only_global() {
    let req = request_json(REQ, "send", "x", "pending");
    let mut c = open(None, vec![card_msg(20, &req)]);
    click(&mut c, ButtonKind::Always);
    let v = c.view();
    let levels = asks(&v)[0].always.clone().unwrap().levels;
    assert_eq!(
        levels.iter().map(|l| l.scope).collect::<Vec<_>>(),
        [RuleScope::Global]
    );
}

#[test]
fn ask_always_cancel_closes_chooser() {
    let mut c = open_with_card();
    click(&mut c, ButtonKind::Always);
    step(&mut c, ChatMsg::AlwaysCancel { request_id: REQ });
    assert!(asks(&c.view())[0].always.is_none());
    assert_eq!(asks(&c.view())[0].state, CardState::Pending);
}
