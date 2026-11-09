//! Watcher (`/danger`), fail-closed, masking secrets in the log, reading the log.

mod chat_common;

use std::time::Duration;

use chat_common::*;
use serde_json::json;

async fn wait_pending(r: &Rig) -> serde_json::Value {
    for _ in 0..500 {
        if let Some(p) = r.d.pending_requests().into_iter().next() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("watcher card did not appear");
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_dangerous_cmd_flow() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/danger rm -rf /srv/data", UUID_A).await;
    let req = wait_pending(&r).await;
    assert_eq!(req["source"], "observer");
    assert_eq!(req["kind"], "dangerous_cmd");
    assert_eq!(req["verdict"], "asked");
    assert_eq!(req["level"], "warn");
    assert_eq!(req["action"], "shell");
    assert_eq!(req["target"], "rm -rf /srv/data");
    assert!(w.event("permission.request", 5).await.is_some());
    let f = r.log_rows("SELECT kind, severity, ref_table, verdict, ref_id FROM observer_findings");
    assert_eq!(f[0][0].as_deref(), Some("dangerous_cmd"));
    assert_eq!(f[0][2].as_deref(), Some("actions_log"));
    assert_eq!(f[0][3].as_deref(), Some("asked"));
    assert_eq!(
        f[0][4],
        Some(req["id"].to_string()),
        "link to the log entry"
    );
    let log = r.log_rows("SELECT decision FROM actions_log WHERE action = 'shell'");
    assert_eq!(
        log[0][0], None,
        "the command was not run before confirmation"
    );
    let id = req["id"].as_i64().unwrap_or(-1);
    assert_eq!(
        r.decide(
            id,
            json!({"decision": "always", "rule": {"scope": "global", "scope_id": 0}})
        )
        .await
        .status,
        422,
        "\"Always\" is not allowed for the watcher"
    );
    assert_eq!(r.decide(id, json!({"decision": "deny"})).await.status, 200);
    let f = r.log_rows("SELECT verdict FROM observer_findings");
    assert_eq!(f[0][0].as_deref(), Some("blocked"));
    let log = r.log_rows("SELECT decision FROM actions_log WHERE action = 'shell'");
    assert_eq!(log[0][0].as_deref(), Some("ask-denied"));
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_unavailable_dangerous_fail_closed() {
    let r = rig().await;
    let s = r.session().await;
    r.d.set_observer_available(false);
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/danger rm -rf /", UUID_A).await;
    let card = w
        .event("permission.request", 10)
        .await
        .unwrap_or_else(|| panic!("no card"));
    assert_eq!(card["request"]["verdict"], "blocked");
    assert_eq!(card["request"]["status"], "denied");
    assert!(
        card["request"]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("watcher unavailable")
    );
    assert!(
        r.d.pending_requests().is_empty(),
        "the action is blocked, nothing to wait for"
    );
    let log = r.log_rows("SELECT decision FROM actions_log WHERE action = 'shell'");
    assert_eq!(log[0][0].as_deref(), Some("deny"));
    let f = r.log_rows("SELECT verdict, detail FROM observer_findings");
    assert_eq!(f[0][0].as_deref(), Some("blocked"));
    r.say(s, "/ask delete file", UUID_B).await;
    let ask = wait_pending(&r).await;
    assert!(
        ask["detail"]
            .as_str()
            .unwrap_or("")
            .contains("watcher unavailable"),
        "a dangerous action comes back as ask with a note"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_masked_in_log_args() {
    let r = rig().await;
    let s = r.session().await;
    let args = r#"{"api_key":"plain-secret-value-aaa","url":"https://x.test","note":"use Bearer abcdefghijklmnop1234 now","nested":{"password":"hunter2"}}"#;
    r.say(s, &format!("/ask read https://x.test {args}"), UUID_A)
        .await;
    for _ in 0..300 {
        let n = r.log_rows("SELECT count(*) FROM actions_log WHERE action = 'read'");
        if n[0][0].as_deref() != Some("0") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let row = r.log_rows("SELECT args_json FROM actions_log WHERE action = 'read'");
    let a = row[0][0].clone().unwrap_or_default();
    assert!(a.contains("***"), "mask is present: {a}");
    for secret in ["plain-secret-value-aaa", "hunter2", "abcdefghijklmnop1234"] {
        assert!(!a.contains(secret), "secret {secret} in args_json");
    }
    assert!(a.contains("https://x.test"), "an ordinary value is intact");
    let all = r.get("/v1/actions?limit=100").await.to_string();
    assert!(
        !all.contains("hunter2") && !all.contains("plain-secret-value"),
        "and in the GET /actions response"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn actions_and_findings_readable() {
    let r = rig().await;
    let s = r.session().await;
    r.say(s, "/ask delete file", UUID_A).await;
    wait_pending(&r).await;
    let list = r
        .get(&format!("/v1/actions?session_id={s}&actor=agent"))
        .await;
    let items = list["items"].as_array().cloned().unwrap_or_default();
    assert!(
        items
            .iter()
            .any(|a| a["action"] == "delete" && a["decision"].is_null()),
        "{list}"
    );
    let none = r.get("/v1/actions?actor=cron").await;
    assert_eq!(none["items"].as_array().map(Vec::len), Some(0));
    let bad = r.call("GET", "/v1/actions?session_id=abc", None).await;
    assert_eq!(bad.status, 400);
}
