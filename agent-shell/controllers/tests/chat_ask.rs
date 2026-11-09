//! S4-07: `ask` cards: buttons, a decision only once, "Always" with a level choice, `409`.

mod chat_common;

use chat_common::*;
use controllers::Controller;
use controllers::chat_types::ReqStatus;
use controllers::chat_view::{ButtonKind, CardState};
use controllers::controllers::chat::ChatMsg;
use controllers::permissions::{Mode, PermDecision, RuleScope};
use serde_json::json;

#[test]
fn ask_card_buttons() {
    let c = open_with_card();
    let v = c.view();
    let cards = asks(&v);
    assert_eq!(cards.len(), 1);
    let labels: Vec<_> = cards[0].buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["Allow", "Deny", "Always"]);
    assert_eq!(cards[0].title, "Permission request: send");
    assert_eq!(cards[0].target, "email to the client");
    assert_eq!(
        (cards[0].state, cards[0].state_label.clone()),
        (CardState::Pending, None)
    );
    assert!(bubbles(&v).is_empty(), "the card is not shown as a bubble");
}

#[test]
fn ask_allow_sends_command() {
    let mut c = open_with_card();
    let fx = click(&mut c, ButtonKind::Allow);
    assert_eq!(commands(&fx), vec![&decide(PermDecision::Allow, None)]);
    let v = c.view();
    assert_eq!(asks(&v)[0].state_label.as_deref(), Some("allowed"));
    assert!(asks(&v)[0].buttons.is_empty());
}

#[test]
fn ask_deny_sends_command() {
    let mut c = open_with_card();
    let fx = click(&mut c, ButtonKind::Deny);
    assert_eq!(commands(&fx), vec![&decide(PermDecision::Deny, None)]);
    assert_eq!(asks(&c.view())[0].state_label.as_deref(), Some("denied"));
}

#[test]
fn ask_card_answered_once() {
    let mut c = open_with_card();
    assert_eq!(commands(&click(&mut c, ButtonKind::Allow)).len(), 1);
    for b in [ButtonKind::Allow, ButtonKind::Deny, ButtonKind::Always] {
        assert!(
            commands(&click(&mut c, b)).is_empty(),
            "{b:?}: no second command is sent"
        );
    }
    let fx = step(
        &mut c,
        ChatMsg::AlwaysPick {
            request_id: REQ,
            scope: RuleScope::Global,
            mode: Mode::Allow,
        },
    );
    assert!(commands(&fx).is_empty());
}

#[test]
fn ask_second_decision_409_shown() {
    let mut c = open_with_card();
    click(&mut c, ButtonKind::Allow);
    let fx = step(
        &mut c,
        ChatMsg::DecisionRejected {
            request_id: REQ,
            status: ReqStatus::Denied,
        },
    );
    assert!(commands(&fx).is_empty());
    let v = c.view();
    assert_eq!(
        asks(&v)[0].state,
        CardState::Denied,
        "the card takes the outcome of whoever decided"
    );
    assert_eq!(asks(&v)[0].state_label.as_deref(), Some("denied"));
    assert!(commands(&click(&mut c, ButtonKind::Allow)).is_empty());
}

#[test]
fn ask_resolved_frame_updates_card() {
    let mut c = open_with_card();
    let resolved = |status: &str| {
        event(
            "permission.resolved",
            json!({ "request_id": REQ, "status": status, "decided_by": "gui", "result": null }),
        )
    };
    frame(&mut c, false, resolved("expired"));
    let v = c.view();
    assert_eq!(asks(&v)[0].state_label.as_deref(), Some("expired"));
    assert!(asks(&v)[0].buttons.is_empty());
    assert!(
        commands(&click(&mut c, ButtonKind::Allow)).is_empty(),
        "no answer is sent after expiry"
    );
}

#[test]
fn ask_decision_failed_returns_to_pending() {
    let mut c = open_with_card();
    click(&mut c, ButtonKind::Allow);
    step(&mut c, ChatMsg::DecisionFailed { request_id: REQ });
    assert_eq!(asks(&c.view())[0].state, CardState::Pending);
    assert_eq!(
        commands(&click(&mut c, ButtonKind::Deny)).len(),
        1,
        "can answer again"
    );
}

#[test]
fn ask_card_already_resolved_in_snapshot_has_no_buttons() {
    let req = request_json(REQ, "delete", "file", "allowed");
    let c = open(None, vec![card_msg(20, &req)]);
    let v = c.view();
    assert!(asks(&v)[0].buttons.is_empty());
    assert_eq!(asks(&v)[0].state_label.as_deref(), Some("allowed"));
}

#[test]
fn ask_card_from_request_frame_without_message() {
    let mut c = open(None, vec![]);
    let req = request_json(REQ, "pay", "invoice", "pending");
    frame(
        &mut c,
        false,
        event("permission.request", json!({ "request": req })),
    );
    assert_eq!(
        asks(&c.view()).len(),
        1,
        "a request without a message is still visible"
    );
}

#[test]
fn ask_card_masks_secrets_in_target_and_detail() {
    let token = format!("agt_{}", "A".repeat(43));
    let mut req = request_json(
        REQ,
        "shell",
        &format!("curl -H 'Authorization: Bearer {token}' x"),
        "pending",
    );
    req["detail"] = json!(format!("token={token}"));
    let c = open(None, vec![card_msg(20, &req)]);
    let v = c.view();
    let card = asks(&v)[0];
    let shown = format!("{} {} {:?}", card.title, card.target, card.detail);
    assert!(
        !shown.contains(&token) && !shown.contains("AAAAAAAA"),
        "secret is visible: {shown}"
    );
}
