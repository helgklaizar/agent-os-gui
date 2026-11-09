//! S4: permission requests: the `ask` scenario, "Always", first-wins, who may decide.

use std::time::Duration;

use serde_json::{Value, json};

use crate::common::{self, Env};

fn cards(msgs: &[Value]) -> usize {
    msgs.iter().filter(|m| m["role"] == "system").count()
}

async fn with_persona(e: &Env, persona_rules: &[(&str, &str)]) -> (i64, i64) {
    let (_, s) = e.model_session().await;
    let p = e.admin.make("/personas", common::persona("P")).await["id"]
        .as_i64()
        .unwrap();
    for (action, mode) in persona_rules {
        let (st, v) = e
            .admin
            .put(
                &format!("/permissions/persona/{p}/{action}"),
                json!({"mode": mode}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let (st, v) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"persona_id": p}))
        .await;
    assert_eq!(st, 200, "{v}");
    (s, p)
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_flow_roundtrip() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    let (st, v) = e.decide(id, json!({"decision": "allow"})).await;
    assert_eq!((st, v["status"].as_str()), (200, Some("allowed")), "{v}");
    let msgs = e.wait_agent_done(s, 2).await;
    let card = msgs.iter().find(|m| m["role"] == "system").unwrap();
    assert_eq!(
        card["meta"]["request"]["status"], "allowed",
        "card resolved"
    );
    assert_eq!(
        e.log_rows("send").await,
        vec![(Some("ask-approved".into()), Some("no_connector".into()))]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_always_persists_permission() {
    let e = Env::start().await;
    // There is no global rule for `shell`; the persona asks for confirmation.
    let (s, p) = with_persona(&e, &[("shell", "ask")]).await;
    e.say(s, "c1", "#scn:action_shell ls").await;
    let (id, _) = e.wait_card(s).await;
    let rule = json!({"scope": "persona", "scope_id": p});
    let (st, v) = e
        .decide(id, json!({"decision": "always", "rule": rule}))
        .await;
    assert_eq!(st, 200, "{v}");
    e.wait_agent_done(s, 2).await;
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=persona&scope_id={p}"))
        .await;
    assert_eq!(
        rows,
        json!([{"scope": "persona", "scope_id": p, "action": "shell", "mode": "allow"}])
    );
    e.say(s, "c2", "#scn:action_shell ls").await;
    let msgs = e.wait_agent_done(s, 4).await;
    assert_eq!(
        cards(&msgs),
        1,
        "the next request at this level does not ask"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_always_stricter_rule_still_applies() {
    let e = Env::start().await;
    let (s, p) = with_persona(&e, &[]).await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    let (st, v) = e
        .chat
        .patch(
            &format!("/sessions/{s}"),
            json!({"cfg": {"perms": {"shell": "ask"}}}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    e.say(s, "c1", "#scn:action_shell ls").await;
    let (id, _) = e.wait_card(s).await;
    let rule = json!({"scope": "persona", "scope_id": p});
    assert_eq!(
        e.decide(id, json!({"decision": "always", "rule": rule}))
            .await
            .0,
        200
    );
    e.wait_agent_done(s, 2).await;
    // A persona rule exists, but a session rule is stricter: a card again.
    e.say(s, "c2", "#scn:action_shell ls").await;
    for _ in 0..200 {
        if cards(&e.messages(s).await) == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(cards(&e.messages(s).await), 2);
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(rows[0]["mode"], "ask", "the session rule is not weakened");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_second_decision_409() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    let path = format!("/permission-requests/{id}/decision");
    let (a, b) = tokio::join!(
        e.decide(id, json!({"decision": "allow"})),
        e.admin2.post(&path, json!({"decision": "deny"})),
    );
    let mut codes = [a.0, b.0];
    codes.sort_unstable();
    assert_eq!(codes, [200, 409], "{a:?} {b:?}");
    let loser = if a.0 == 409 { &a.1 } else { &b.1 };
    assert_eq!(loser["error"]["details"]["reason"], "already_decided");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_decision_external_token_403() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    for api in [&e.chat, &e.read] {
        let (st, _) = api
            .post(
                &format!("/permission-requests/{id}/decision"),
                json!({"decision": "allow"}),
            )
            .await;
        assert_eq!(st, 403, "an external token may not decide");
    }
    let (_, r) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(r["status"], "pending");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_token_cannot_decide() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    for api in [&e.chat, &e.agent, &e.module] {
        let (st, _) = api
            .post(
                &format!("/permission-requests/{id}/decision"),
                json!({"decision": "allow"}),
            )
            .await;
        assert_eq!(st, 403);
    }
    let rows = e
        .inspect(move |c| {
            c.query_row(
                "SELECT decision FROM actions_log WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
        })
        .await;
    assert_eq!(rows, None, "journal without a decision");
}
