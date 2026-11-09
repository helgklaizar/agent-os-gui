//! S4-08: observer cards: red border, blocked until confirmed, the decision goes to the core.

mod chat_common;

use chat_common::*;
use controllers::Controller;
use controllers::chat_view::{
    ButtonKind, CardState, ChatItem, ChatView, ObserverCard, ObserverKind, Verdict,
};
use controllers::controllers::chat::ChatController;
use controllers::permissions::PermDecision;
use serde_json::{Value, json};

fn observer_req(kind: &str, verdict: &str, target: &str, status: &str) -> Value {
    let mut r = request_json(REQ, "shell", target, status);
    r["source"] = json!("observer");
    r["kind"] = json!(kind);
    r["verdict"] = json!(verdict);
    r
}

fn open_obs(req: Value) -> ChatController {
    open(Some(WORK), vec![card_msg(20, &req)])
}

fn cards(v: &ChatView) -> Vec<&ObserverCard> {
    v.items
        .iter()
        .filter_map(|i| match i {
            ChatItem::Observer(o) => Some(o),
            _ => None,
        })
        .collect()
}

#[test]
fn observer_card_red_border() {
    let c = open_obs(observer_req(
        "dangerous_cmd",
        "asked",
        "rm -rf /home/u",
        "pending",
    ));
    let v = c.view();
    let o = cards(&v);
    assert_eq!(o.len(), 1);
    assert!(o[0].red_border);
    assert_eq!(
        (o[0].kind, o[0].kind_label.as_str()),
        (ObserverKind::DangerousCmd, "Dangerous command")
    );
    assert_eq!(o[0].target, "rm -rf /home/u");
    assert!(asks(&v).is_empty(), "an observer card is not a plain ask");
    for (kind, want) in [
        ("injection", "Suspected injection"),
        ("phishing", "Suspected phishing"),
    ] {
        let c = open_obs(observer_req(kind, "blocked", "x", "pending"));
        assert_eq!(cards(&c.view())[0].kind_label, want);
        assert!(
            cards(&c.view())[0].red_border,
            "the border is red for any verdict"
        );
    }
}

#[test]
fn observer_blocked_until_confirm() {
    let mut c = open_obs(observer_req(
        "dangerous_cmd",
        "asked",
        "rm -rf /home/u",
        "pending",
    ));
    let v = c.view();
    let o = cards(&v)[0];
    assert!(o.awaiting, "the action is blocked until confirmed");
    assert_eq!(o.verdict, Verdict::Asked);
    let labels: Vec<_> = o.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["Confirm", "Reject"]);
    let fx = click(&mut c, ButtonKind::Confirm);
    assert_eq!(commands(&fx), vec![&decide(PermDecision::Allow, None)]);
    let v = c.view();
    assert!(!cards(&v)[0].awaiting && cards(&v)[0].buttons.is_empty());
    assert_eq!(cards(&v)[0].state, CardState::Allowed);
    assert!(
        commands(&click(&mut c, ButtonKind::Reject)).is_empty(),
        "the decision is sent once"
    );
}

#[test]
fn observer_reject_sends_deny() {
    let mut c = open_obs(observer_req("phishing", "asked", "email", "pending"));
    let fx = click(&mut c, ButtonKind::Reject);
    assert_eq!(commands(&fx), vec![&decide(PermDecision::Deny, None)]);
    assert_eq!(cards(&c.view())[0].state, CardState::Denied);
}

#[test]
fn observer_blocked_and_allowed_verdicts_have_no_buttons() {
    for (verdict, label) in [
        ("blocked", "blocked"),
        ("allowed", "allowed by the observer"),
    ] {
        let c = open_obs(observer_req("injection", verdict, "x", "pending"));
        let v = c.view();
        assert!(cards(&v)[0].buttons.is_empty(), "{verdict}");
        assert!(!cards(&v)[0].awaiting, "{verdict}");
        assert_eq!(cards(&v)[0].verdict_label, label);
    }
}

#[test]
fn observer_card_has_no_always() {
    let mut c = open_obs(observer_req("dangerous_cmd", "asked", "x", "pending"));
    assert!(commands(&click(&mut c, ButtonKind::Always)).is_empty());
    assert_eq!(
        cards(&c.view())[0].buttons.len(),
        2,
        "\"Always\" is not offered for the observer"
    );
}

#[test]
fn observer_card_text_is_literal_and_masked() {
    let token = format!("agt_{}", "B".repeat(43));
    let evil = format!("<img src=x onerror=alert(1)> token={token}");
    let c = open_obs(observer_req("dangerous_cmd", "asked", &evil, "pending"));
    let v = c.view();
    let t = &cards(&v)[0].target;
    assert!(
        t.starts_with("<img src=x onerror=alert(1)> token="),
        "HTML is literal: {t}"
    );
    assert!(!t.contains(&token), "the secret is not shown: {t}");
}
