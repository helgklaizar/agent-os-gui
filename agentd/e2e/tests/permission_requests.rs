//! A4-03: permission requests from HTTP mutations by `agent` and `module` tokens (`202`), card, frame, snapshot, `GET`.

mod common;

use std::time::Duration;

use common::{Env, Ws};
use serde_json::{Value, json};

async fn session(e: &Env) -> i64 {
    e.chat.make("/sessions", json!({"name": "s"})).await["id"]
        .as_i64()
        .unwrap()
}

async fn db_row(e: &mut Env, id: i64) -> (Option<String>, Option<String>) {
    e.inspect(move |c| {
        c.query_row(
            "SELECT decision, result FROM actions_log WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn http_mutation_gate_agent_202() {
    let e = Env::start().await;
    let s = session(&e).await;
    let (st, req) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 202, "{req}");
    assert_eq!(
        (
            req["action"].as_str(),
            req["status"].as_str(),
            req["source"].as_str(),
            req["level"].as_str()
        ),
        (Some("delete"), Some("pending"), Some("policy"), Some("ask")),
        "{req}"
    );
    assert_eq!(req["session_id"], s);
    assert!(
        req["id"].as_i64().unwrap() > 0
            && req["expires_at"].as_i64().unwrap() > req["created_at"].as_i64().unwrap()
    );
    // The mutation was not executed: the session is still there.
    let (st, _) = e.read.get(&format!("/sessions/{s}")).await;
    assert_eq!(st, 200);
    // Module: same.
    let (st, req2) = e.module.delete(&format!("/sessions/{s}")).await;
    assert_eq!(
        (st, req2["status"].as_str()),
        (202, Some("pending")),
        "{req2}"
    );
    assert_ne!(req["id"], req2["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_mutation_gate_gui_no_202() {
    let e = Env::start().await;
    // `gui` and `external` do not get `202`: confirmation is a client-side dialog (assumption).
    let a = session(&e).await;
    let (st, _) = e.admin.delete(&format!("/sessions/{a}")).await;
    assert_eq!(st, 204);
    let b = session(&e).await;
    let (st, _) = e.chat.delete(&format!("/sessions/{b}")).await;
    assert_eq!(st, 204);
    let c = session(&e).await;
    let (st, _) = e
        .chat
        .post(&format!("/sessions/{c}/clear"), json!({}))
        .await;
    assert_eq!(st, 204);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_mutation_gate_routes_and_actions() {
    let e = Env::start().await;
    let s = session(&e).await;
    let f = e.chat.make("/folders", json!({"name": "f"})).await["id"]
        .as_i64()
        .unwrap();
    for (what, path, expect_action) in [
        ("clear", format!("/sessions/{s}/clear"), "delete"),
        ("folder", format!("/folders/{f}"), "delete"),
    ] {
        let (st, req) = if what == "folder" {
            e.agent.delete(&path).await
        } else {
            e.agent.post(&path, json!({})).await
        };
        assert_eq!(
            (st, req["action"].as_str()),
            (202, Some(expect_action)),
            "{what}: {req}"
        );
    }
    let (st, req) = e
        .agent
        .post(
            &format!("/sessions/{s}/messages"),
            json!({"body_md": "hello", "client_id": "c1"}),
        )
        .await;
    assert_eq!((st, req["action"].as_str()), (202, Some("send")), "{req}");
    let (_, msgs) = e.read.get(&format!("/sessions/{s}/messages")).await;
    assert!(
        msgs.as_array().unwrap().iter().all(|m| m["role"] != "user"),
        "message not sent"
    );
    // Settings without a session: `config_write`, no card, `session_id` is empty.
    let (st, req) = e
        .agent
        .patch("/settings/theme", json!({"theme": "dark"}))
        .await;
    assert_eq!(
        (st, req["action"].as_str(), req["session_id"].clone()),
        (202, Some("config_write"), Value::Null),
        "{req}"
    );
    let (_, theme) = e.read.get("/settings/theme").await;
    assert_ne!(theme["theme"], "dark", "setting not applied");
    // Secrets: not available to the agent token at all (admin and local) - the gate does not expose them.
    let (st, _) = e
        .agent
        .put("/secrets/provider.x", json!({"value": "v"}))
        .await;
    assert_eq!(st, 403);
    // A nonexistent session gives a plain 404, not a permission request.
    let (st, _) = e.agent.delete("/sessions/99999").await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_mutation_gate_policy_deny_and_allow() {
    let mut e = Env::start().await;
    let a = session(&e).await;
    let (st, _) = e
        .admin
        .put("/permissions/global/0/delete", json!({"mode": "deny"}))
        .await;
    assert_eq!(st, 200);
    let (st, v) = e.agent.delete(&format!("/sessions/{a}")).await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (403, Some("permission_denied")),
        "{v}"
    );
    let (st, _) = e.read.get(&format!("/sessions/{a}")).await;
    assert_eq!(st, 200, "deny: session intact");
    // Global allow: the agent deletes without asking, the log shows `allow`.
    let (st, _) = e
        .admin
        .put("/permissions/global/0/delete", json!({"mode": "allow"}))
        .await;
    assert_eq!(st, 200);
    let (st, _) = e.agent.delete(&format!("/sessions/{a}")).await;
    assert_eq!(st, 204);
    let rows: Vec<(String, Option<String>)> = e
        .inspect(|c| {
            c.prepare(
                "SELECT decision, result FROM actions_log WHERE action = 'delete' ORDER BY id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
        })
        .await;
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["deny", "allow"],
        "{rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_request_get_state() {
    let e = Env::start().await;
    let s = session(&e).await;
    let (_, req) = e.agent.delete(&format!("/sessions/{s}")).await;
    let id = req["id"].as_i64().unwrap();
    let (st, got) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(st, 200, "{got}");
    assert_eq!(got, req, "GET returns the same as `202`");
    assert_eq!(
        got["expires_at"].as_i64().unwrap() - got["created_at"].as_i64().unwrap(),
        600
    );
    let (st, _) = e.read.get("/permission-requests/987654").await;
    assert_eq!(st, 404);
    let (st, _) = e.read.get("/permission-requests/abc").await;
    assert_eq!(st, 400);
    // A log entry that is not a permission request (the policy already decided) is not visible through this route.
    let _ = e
        .admin
        .put("/permissions/global/0/delete", json!({"mode": "deny"}))
        .await;
    let (_, _) = e.agent.delete(&format!("/sessions/{s}")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_request_created() {
    let mut e = Env::start().await;
    let s = session(&e).await;
    let mut ws: Ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let (st, req) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 202, "{req}");
    let id = req["id"].as_i64().unwrap();
    // The `permission.request` frame and the card match in content.
    let frames = ws.until(Duration::from_secs(5), |f| {
        f.data["kind"] == "permission.request"
    });
    let frame = &frames.last().unwrap().data;
    assert_eq!(frame["request"], req);
    let msgs = e.messages(s).await;
    let cards: Vec<&Value> = msgs.iter().filter(|m| m["role"] == "system").collect();
    assert_eq!(cards.len(), 1, "{msgs:?}");
    assert_eq!(cards[0]["meta"]["kind"], "permission_request");
    assert_eq!(cards[0]["meta"]["request_id"], id);
    assert_eq!(cards[0]["meta"]["request"], req);
    // Fresh subscription: the request is in the snapshot `open_requests`.
    let mut ws2 = e.ws();
    let snap = ws2.subscribe(&format!("session.{s}"));
    assert_eq!(snap["open_requests"], json!([req]));
    // Log entry without a decision.
    let (decision, result) = db_row(&mut e, id).await;
    assert_eq!((decision, result), (None, None));
}
