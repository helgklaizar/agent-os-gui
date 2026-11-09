//! A4-03 (DB): `PolicyGate` on real stores: permission modes, permission request, card, frame, snapshot.

mod gate_rig;

use agentd_core::{Action, ChatStore, Gate, Level, Mode, Status};
use gate_rig::{ask, input, rig, row, set};

#[tokio::test]
async fn gate_allow_without_rules_logs_allow() {
    let r = rig().await;
    let Gate::Allow { action_id } = r.gate.check(input(Action::Shell, "ls -la")).await.unwrap()
    else {
        panic!("no rule: the action is not restricted by permissions");
    };
    let (decision, _, args) = row(&r, action_id).await;
    assert_eq!(decision.as_deref(), Some("allow"));
    assert!(!args.unwrap().contains("tok-VALUE-1"), "secret in the log");
}

#[tokio::test]
async fn ask_request_created() {
    let r = rig().await;
    let req = ask(r
        .gate
        .check(input(Action::Send, "mail:friend@example.test"))
        .await
        .unwrap());
    assert_eq!(
        (req.status, req.source.as_str(), req.level.as_str()),
        (Status::Pending, "policy", "ask")
    );
    assert_eq!(req.expires_at, Some(req.created_at + 600));
    // A log entry without a decision; `args_json` has no secret.
    let (decision, result, args) = row(&r, req.id).await;
    assert_eq!((decision, result), (None, None));
    assert!(!args.unwrap().contains("tok-VALUE-1"));
    // The chat card and the frame match in content.
    let msgs = r.chat.list(1, None, 50).await.unwrap();
    let card = &msgs[0];
    assert_eq!(
        (card["role"].as_str(), card["meta"]["kind"].as_str()),
        (Some("system"), Some("permission_request"))
    );
    assert_eq!(card["meta"]["request_id"], req.id);
    let frames = r.events.0.lock().unwrap().clone();
    let frame = frames
        .iter()
        .find(|(t, d)| t == "session.1" && d["kind"] == "permission.request")
        .expect("permission.request frame");
    assert_eq!(frame.1["request"], card["meta"]["request"]);
    assert_eq!(frame.1["request"]["id"], req.id);
    // A row in the snapshot's `open_requests`.
    let snap = r.chat.session_snapshot(1).await.unwrap();
    let open = snap["open_requests"].as_array().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0], card["meta"]["request"]);
}

#[tokio::test]
async fn gate_policy_modes_and_tightening() {
    let r = rig().await;
    set(&r, Level::Global, 0, Action::Send, Mode::Deny).await;
    let Gate::Deny { action_id, reason } = r.gate.check(input(Action::Send, "x")).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(reason, "policy_deny");
    let (decision, result, _) = row(&r, action_id).await;
    assert_eq!(
        (decision.as_deref(), result.as_deref()),
        (Some("deny"), Some("policy_deny"))
    );
    // Global allow, but the session tightens to ask: it asks.
    set(&r, Level::Global, 0, Action::Pay, Mode::Allow).await;
    assert!(matches!(
        r.gate.check(input(Action::Pay, "p")).await.unwrap(),
        Gate::Allow { .. }
    ));
    set(&r, Level::Session, 1, Action::Pay, Mode::Ask).await;
    ask(r.gate.check(input(Action::Pay, "p")).await.unwrap());
}
