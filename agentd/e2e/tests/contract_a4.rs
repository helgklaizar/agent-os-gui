//! A4-11: `contract_a4_routes` checks A4 route replies against the `openapi-draft.yaml` schemas.

mod common;
mod openapi;

use common::Env;
use openapi::{check, declared, schemas};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn contract_a4_routes() {
    let all = schemas();
    for name in [
        "Permission",
        "PermissionRequest",
        "PermissionDecision",
        "ToolResult",
        "ToolResultAck",
    ] {
        assert!(!all[name].props.is_empty(), "schema {name} was not parsed");
    }
    let e = Env::start().await;
    let mut seen: Vec<(&str, &str, u16)> = Vec::new();
    let mut rec = |m: &'static str, p: &'static str, st: u16| seen.push((m, p, st));

    // Permissions.
    let (st, rules) = e.read.get("/permissions").await;
    rec("get", "/permissions", st);
    assert_eq!(st, 200);
    rules
        .as_array()
        .unwrap()
        .iter()
        .for_each(|p| check("Permission", p, &all));
    let (st, rule) = e
        .admin
        .put("/permissions/global/0/shell", json!({"mode": "ask"}))
        .await;
    rec("put", "/permissions/{scope}/{scope_id}/{action}", st);
    check("Permission", &rule, &all);
    let s = e.chat.make("/sessions", json!({"name": "s"})).await["id"]
        .as_i64()
        .unwrap();
    let (st, err) = e
        .admin
        .put(
            &format!("/permissions/session/{s}/send"),
            json!({"mode": "allow"}),
        )
        .await;
    rec("put", "/permissions/{scope}/{scope_id}/{action}", st);
    assert_eq!(
        (st, err["error"]["code"].as_str()),
        (403, Some("permission_denied"))
    );
    let (st, _) = e
        .admin
        .put("/permissions/global/0/teleport", json!({"mode": "ask"}))
        .await;
    rec("put", "/permissions/{scope}/{scope_id}/{action}", st);
    assert_eq!(st, 422);
    // Permission request (`202` for the agent token), state, decision.
    let (st, req) = e.agent.delete(&format!("/sessions/{s}")).await;
    rec("delete", "/sessions/{id}", st);
    assert_eq!(st, 202);
    check("PermissionRequest", &req, &all);
    let id = req["id"].as_i64().unwrap();
    let (st, got) = e.read.get(&format!("/permission-requests/{id}")).await;
    rec("get", "/permission-requests/{id}", st);
    check("PermissionRequest", &got, &all);
    let (st, _) = e
        .chat
        .post(
            &format!("/permission-requests/{id}/decision"),
            json!({"decision": "allow"}),
        )
        .await;
    rec("post", "/permission-requests/{id}/decision", st);
    assert_eq!(st, 403);
    let (st, done) = e.decide(id, json!({"decision": "allow"})).await;
    rec("post", "/permission-requests/{id}/decision", st);
    assert_eq!(st, 200);
    check("PermissionRequest", &done, &all);
    let (st, err) = e.decide(id, json!({"decision": "deny"})).await;
    rec("post", "/permission-requests/{id}/decision", st);
    assert_eq!((st, err["error"]["code"].as_str()), (409, Some("conflict")));
    let (st, _) = e.decide(id, json!({"decision": "always"})).await;
    rec("post", "/permission-requests/{id}/decision", st);
    assert_eq!(st, 422);
    let (st, _) = e.read.get("/permission-requests/999999").await;
    rec("get", "/permission-requests/{id}", st);
    assert_eq!(st, 404);
    // `tool-results`: browser command, `202` and errors.
    let (_, ms) = e.model_session().await;
    let mut ws = e.ws_as(&e.chat_token);
    ws.subscribe(&format!("session.{ms}"));
    e.say(ms, "c1", "#scn:action_browse open").await;
    let frames = ws.until(std::time::Duration::from_secs(10), |f| {
        f.data["kind"] == "browser.command"
    });
    let rid = frames.last().unwrap().data["request_id"]
        .as_str()
        .unwrap()
        .to_string();
    let body = json!({"request_id": rid, "kind": "browser", "status": "ok", "output": "x"});
    let (st, ack) = e
        .chat
        .post(&format!("/sessions/{ms}/tool-results"), body.clone())
        .await;
    rec("post", "/sessions/{id}/tool-results", st);
    assert_eq!(st, 202);
    check("ToolResultAck", &ack, &all);
    let (st, _) = e
        .chat
        .post(
            &format!("/sessions/{ms}/tool-results"),
            json!({"request_id": rid, "kind": "browser", "status": "ok", "output": "y"}),
        )
        .await;
    rec("post", "/sessions/{id}/tool-results", st);
    assert_eq!(st, 409);
    let (st, _) = e
        .chat
        .post(&format!("/sessions/{ms}/tool-results"), json!({}))
        .await;
    rec("post", "/sessions/{id}/tool-results", st);
    assert_eq!(st, 422);

    assert!(seen.len() >= 14, "{} calls", seen.len());
    for (method, path, st) in &seen {
        let ok = declared(path, method);
        assert!(
            !ok.is_empty(),
            "{method} {path}: operation is not in the specification"
        );
        assert!(
            ok.contains(st) || matches!(st, 403 | 404 | 409 | 422),
            "{method} {path} → {st}, declared {ok:?}"
        );
    }
}
