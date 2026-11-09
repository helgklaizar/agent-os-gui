//! Permission requests, "Always", 409, 403, expiry, weakening, journal: the client against `fake-agentd`.

mod common;

use common::{http, issue, start};
use controllers::permissions::{Mode, PermDecision, RuleScope, RuleSpec};
use fake_agentd::{Config, FakeAgentd, Kind, Scope};
use serde_json::Value;
use std::time::Duration;
use ui::chat_exec::decide;

const SESSION: i64 = 1;

fn uuid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn ask(d: &FakeAgentd, text: &str) -> i64 {
    let h = http(d, Some(d.admin_token()));
    assert!(h.send_message(SESSION, &uuid(), text, &[]).await.is_ok());
    for _ in 0..100 {
        if let Some(id) = d
            .pending_requests()
            .first()
            .map(|r| r["id"].as_i64().unwrap_or(0))
        {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the permission request did not appear");
}

async fn actions(d: &FakeAgentd, session: Option<i64>) -> Vec<Value> {
    let h = http(d, Some(d.admin_token()));
    let page = h.list_actions(session).await.unwrap_or_default();
    page["items"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn ask_flow_roundtrip() {
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask send a@example.org").await;
    let h = http(&d, Some(d.admin_token()));
    assert!(decide(&h, id, PermDecision::Allow, None).await.is_ok());
    let r = h.permission_request(id).await.unwrap_or_default();
    assert_eq!(r["status"], "allowed");
    let log = actions(&d, None).await;
    assert!(
        log.iter()
            .any(|a| a["id"] == id && a["decision"] == "ask-approved"),
        "{log:?}"
    );
}

#[tokio::test]
async fn ask_second_decision_409() {
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask delete x").await;
    let h = http(&d, Some(d.admin_token()));
    assert!(decide(&h, id, PermDecision::Deny, None).await.is_ok());
    let second = decide(&h, id, PermDecision::Allow, None).await;
    assert_eq!(
        second.err().and_then(|e| e.api().map(|a| a.status)),
        Some(409)
    );
}

#[tokio::test]
async fn ask_decision_external_token_403() {
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask send a@example.org").await;
    for (scope, kind) in [(Scope::Chat, Kind::External), (Scope::Chat, Kind::Gui)] {
        let h = http(&d, Some(&issue(&d, scope, kind)));
        let r = decide(&h, id, PermDecision::Allow, None).await;
        assert_eq!(
            r.err().and_then(|e| e.api().map(|a| a.status)),
            Some(403),
            "{scope:?} {kind:?}"
        );
    }
    assert_eq!(d.pending_requests().len(), 1, "the request stays pending");
}

#[tokio::test]
async fn ask_always_persists_permission_and_stricter_rule_wins() {
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask pay shop").await;
    let h = http(&d, Some(d.admin_token()));
    let rule = RuleSpec {
        scope: RuleScope::Persona,
        scope_id: 2,
        mode: Mode::Allow,
    };
    assert!(
        decide(&h, id, PermDecision::Always, Some(rule))
            .await
            .is_ok()
    );
    let rows = h.list_permissions("persona", 2).await.unwrap_or_default();
    assert!(
        rows.iter()
            .any(|r| r["action"] == "pay" && r["mode"] == "allow"),
        "{rows:?}"
    );
}

#[tokio::test]
async fn weaken_attempt_logged() {
    let d = start(Config::temp().no_ticker()).await;
    assert!(d.seed_permission("global", 0, "shell", "deny").is_ok());
    let h = http(&d, Some(d.admin_token()));
    let r = h.put_permission("session", 1, "shell", "allow").await;
    assert_eq!(r.err().and_then(|e| e.api().map(|a| a.status)), Some(403));
    let body = h
        .list_findings(Some("permission_loosen"))
        .await
        .unwrap_or_default();
    assert!(
        body["items"].as_array().is_some_and(|a| !a.is_empty()),
        "{body}"
    );
}

#[tokio::test]
async fn session_delete_removes_messages() {
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask send a@example.org").await;
    let h = http(&d, Some(d.admin_token()));
    assert!(decide(&h, id, PermDecision::Allow, None).await.is_ok());
    assert!(h.delete_session(SESSION).await.is_ok());
    let gone = h.list_messages(SESSION, None, 50).await;
    assert_eq!(
        gone.err().and_then(|e| e.api().map(|a| a.status)),
        Some(404)
    );
    let log = actions(&d, None).await;
    let row = log.iter().find(|a| a["id"] == id);
    assert!(row.is_some_and(|a| a["session_id"].is_null()), "{log:?}");
}

#[tokio::test]
async fn decide_outcome_maps_409_to_winner_status() {
    use ui::chat_exec::{DecisionOutcome, decide_outcome};
    let d = start(Config::temp().no_ticker()).await;
    let id = ask(&d, "/ask delete x").await;
    let h = http(&d, Some(d.admin_token()));
    assert_eq!(
        decide_outcome(&h, id, PermDecision::Deny, None).await,
        DecisionOutcome::Done
    );
    let second = decide_outcome(&h, id, PermDecision::Allow, None).await;
    assert_eq!(
        second,
        DecisionOutcome::Rejected(controllers::chat_types::ReqStatus::Denied)
    );
    let missing = decide_outcome(&h, 99_999, PermDecision::Allow, None).await;
    assert_eq!(
        missing,
        DecisionOutcome::Failed,
        "404 is not 'someone else decided'"
    );
}
