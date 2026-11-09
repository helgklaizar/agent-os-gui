//! A-core-100 (A4-13 item 2): two decisions on one request at once — one wins, and the "Always" rule remains only
//! for a winning "Always"; a losing "Always" writes no rule.

mod common;

use common::{Env, ask_delete, decide, session};
use serde_json::json;

fn always() -> serde_json::Value {
    json!({"decision": "always", "rule": {"scope": "global", "scope_id": 0}})
}

async fn global_delete_rule(e: &Env) -> bool {
    let (_, rows) = e.read.get("/permissions?scope=global").await;
    rows.as_array()
        .unwrap()
        .iter()
        .any(|p| p["action"] == "delete" && p["mode"] == "allow")
}

#[tokio::test(flavor = "multi_thread")]
async fn always_vs_deny_race_leaves_rule_only_for_winner() {
    for round in 0..6 {
        let e = Env::start().await;
        let s = session(&e, None).await;
        let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
        let (a, d) = tokio::join!(
            decide(&e.admin, id, always()),
            decide(&e.admin, id, json!({"decision": "deny"}))
        );
        let codes = [a.0, d.0];
        assert!(
            codes.contains(&200) && codes.contains(&409),
            "round {round}: one won, the other got 409: {codes:?}"
        );
        let always_won = a.0 == 200;
        assert_eq!(
            global_delete_rule(&e).await,
            always_won,
            "round {round}: the rule exists only if \"Always\" won ({a:?} / {d:?})"
        );
        let (_, req) = e.admin.get(&format!("/permission-requests/{id}")).await;
        assert_eq!(
            req["status"],
            if always_won { "allowed" } else { "denied" },
            "round {round}: {req}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_always_race_writes_one_rule_one_winner() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (a, b) = tokio::join!(
        decide(&e.admin, id, always()),
        decide(&e.admin, id, always())
    );
    let mut codes = [a.0, b.0];
    codes.sort_unstable();
    assert_eq!(codes, [200, 409], "{a:?} {b:?}");
    assert!(global_delete_rule(&e).await);
}
