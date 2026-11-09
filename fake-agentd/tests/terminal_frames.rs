//! Agent terminal in `fake-agentd`: `terminal.*` frames (api.md section 4.4) for agent commands under the `shell` permission.
//! The "core" runs the command; a dangerous one waits for a decision, a refusal and an unavailable watcher give `terminal.status = denied`.

mod chat_common;

use chat_common::*;
use serde_json::{Value, json};

fn terminal(w: &Watch) -> Vec<Value> {
    w.all()
        .into_iter()
        .filter(|f| {
            f["data"]["kind"]
                .as_str()
                .is_some_and(|k| k.starts_with("terminal."))
        })
        .map(|f| f["data"].clone())
        .collect()
}

fn statuses(w: &Watch) -> Vec<String> {
    terminal(w)
        .iter()
        .filter(|f| f["kind"] == "terminal.status")
        .filter_map(|f| f["status"].as_str().map(str::to_string))
        .collect()
}

async fn ready(r: &Rig, s: i64) -> Watch {
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    assert!(
        w.until(3, |w| (!w.all().is_empty()).then_some(()))
            .await
            .is_some()
    );
    w
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_shell_allowed_emits_command_output_status() {
    let r = rig().await;
    let s = r.session().await;
    assert_eq!(r.put_rule("global", 0, "shell", "allow").await.status, 200);
    let w = ready(&r, s).await;
    r.say(s, "/ask shell ls -la", UUID_A).await;
    let done = w
        .until(5, |w| {
            statuses(w).contains(&"exited".to_string()).then_some(())
        })
        .await;
    assert!(done.is_some(), "no exited: {:?}", terminal(&w));
    let frames = terminal(&w);
    let command = frames
        .iter()
        .find(|f| f["kind"] == "terminal.command")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(command["command"], "ls -la");
    let rid = command["request_id"].as_str().unwrap_or("").to_string();
    assert!(!rid.is_empty());
    assert!(frames.iter().all(|f| f["request_id"] == rid.as_str()));
    assert_eq!(statuses(&w), vec!["started", "exited"]);
    let out: Vec<&Value> = frames
        .iter()
        .filter(|f| f["kind"] == "terminal.output")
        .collect();
    assert!(!out.is_empty(), "there is output");
    assert_eq!(out[0]["stream"], "stdout");
    assert_eq!(out[0]["seq"], 0);
    let exited = frames
        .iter()
        .find(|f| f["status"] == "exited")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(exited["exit_code"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn dangerous_command_blocked_until_approved() {
    let r = rig().await;
    let s = r.session().await;
    let w = ready(&r, s).await;
    let req = ask(&r, s, "/danger rm -rf /tmp/x").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        terminal(&w).is_empty(),
        "before the decision the core runs nothing"
    );
    let ok = r.decide(id, json!({"decision": "allow"})).await;
    assert_eq!(ok.status, 200);
    let done = w
        .until(5, |w| {
            statuses(w).contains(&"exited".to_string()).then_some(())
        })
        .await;
    assert!(done.is_some(), "after \"Allow\" the command ran");
    let again = r.decide(id, json!({"decision": "allow"})).await;
    assert_eq!(again.status, 409, "the second answer - first-wins");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        statuses(&w).iter().filter(|s| *s == "started").count(),
        1,
        "ran once"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dangerous_command_rejected_never_runs() {
    let r = rig().await;
    let s = r.session().await;
    let w = ready(&r, s).await;
    let req = ask(&r, s, "/danger git push --force").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    let no = r.decide(id, json!({"decision": "deny"})).await;
    assert_eq!(no.status, 200);
    let denied = w
        .until(5, |w| {
            statuses(w).contains(&"denied".to_string()).then_some(())
        })
        .await;
    assert!(denied.is_some(), "{:?}", terminal(&w));
    assert!(!statuses(&w).contains(&"started".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_unavailable_blocks_dangerous() {
    let r = rig().await;
    let s = r.session().await;
    r.d.set_observer_available(false);
    let w = ready(&r, s).await;
    r.say(s, "/danger rm -rf /", UUID_A).await;
    let denied = w
        .until(5, |w| {
            statuses(w).contains(&"denied".to_string()).then_some(())
        })
        .await;
    assert!(denied.is_some(), "fail-closed: denied without running");
    assert!(!statuses(&w).contains(&"started".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_shell_deny_rule_emits_denied() {
    let r = rig().await;
    let s = r.session().await;
    assert_eq!(r.put_rule("global", 0, "shell", "deny").await.status, 200);
    let w = ready(&r, s).await;
    r.say(s, "/ask shell ls", UUID_A).await;
    let denied = w
        .until(5, |w| {
            statuses(w).contains(&"denied".to_string()).then_some(())
        })
        .await;
    assert!(denied.is_some(), "{:?}", terminal(&w));
    assert_eq!(statuses(&w), vec!["denied"]);
}
