//! Sending, history, `client_id` idempotency, attachments, changing the session persona and folder.

mod chat_common;

use chat_common::*;
use fake_agentd::{Kind, Scope};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn send_message_stored_and_listed_newest_first() {
    let r = rig().await;
    let s = r.session().await;
    let first = r.say(s, "hello", UUID_A).await;
    assert_eq!(
        first.status,
        201,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let m = first.json();
    assert_eq!(m["role"], "user");
    assert_eq!(m["body_md"], "hello");
    assert_eq!(m["client_id"], UUID_A);
    assert_eq!(m["status"], "done");
    assert_eq!(m["session_id"], s);
    assert!(m["id"].is_i64() && m["ts"].is_i64());
    let second = r.say(s, "more", UUID_B).await;
    assert_eq!(second.status, 201);
    let list = r.messages(s).await;
    let users: Vec<_> = list.iter().filter(|x| x["role"] == "user").collect();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0]["body_md"], "more", "newest first");
    let id = users[0]["id"].as_i64().unwrap_or(0);
    let page = r
        .get(&format!("/v1/sessions/{s}/messages?before={id}&limit=1"))
        .await;
    let page = page.as_array().cloned().unwrap_or_default();
    assert_eq!(page.len(), 1);
    assert!(
        page[0]["id"].as_i64().unwrap_or(i64::MAX) < id,
        "before cursor"
    );
    let rows = r.log_rows(&format!(
        "SELECT count(*) FROM messages WHERE session_id = {s} AND role = 'user'"
    ));
    assert_eq!(rows[0][0].as_deref(), Some("2"), "rows in messages");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_duplicate_client_id_idempotent() {
    let r = rig().await;
    let s = r.session().await;
    let a = r.say(s, "one", UUID_A).await;
    assert_eq!(a.status, 201);
    let b = r.say(s, "one", UUID_A).await;
    assert_eq!(
        b.status, 200,
        "a repeat returns the already created message"
    );
    assert_eq!(a.json()["id"], b.json()["id"]);
    let c = r.say(s, "other text", UUID_A).await;
    assert_eq!(c.status, 409);
    assert_eq!(code(&c), "client_id_conflict");
    let users = r
        .messages(s)
        .await
        .into_iter()
        .filter(|x| x["role"] == "user")
        .count();
    assert_eq!(users, 1, "one message");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_rejects_bad_input() {
    let r = rig().await;
    let s = r.session().await;
    assert_eq!(
        r.say(s, "x", "not-a-uuid").await.status,
        422,
        "foreign client_id format"
    );
    let no_body = r
        .call(
            "POST",
            &format!("/v1/sessions/{s}/messages"),
            Some(json!({"client_id": UUID_A})),
        )
        .await;
    assert_eq!(no_body.status, 422);
    assert_eq!(r.say(99_999, "x", UUID_A).await.status, 404);
    let read = r.token(Scope::Read, Kind::Test);
    let denied = r
        .call_as(
            &read,
            "POST",
            &format!("/v1/sessions/{s}/messages"),
            Some(json!({"body_md": "x", "client_id": UUID_A})),
        )
        .await;
    assert_eq!(denied.status, 403);
    assert_eq!(code(&denied), "insufficient_scope");
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_upload_download_and_meta() {
    let r = rig().await;
    let s = r.session().await;
    let evil = "<img src=x onerror=alert(1)>.txt";
    let (ct, body) = multipart(evil, "text/plain", b"secret-file-body");
    let up = send(
        r.d.socket_path(),
        Req::new("POST", &format!("/v1/sessions/{s}/attachments"))
            .token(&r.admin)
            .raw(&ct, body),
    )
    .await;
    assert_eq!(up.status, 201, "{}", String::from_utf8_lossy(&up.body));
    let a = up.json();
    assert_eq!(a["name"], evil);
    assert_eq!(a["size"], 16);
    assert_eq!(a["mime"], "text/plain");
    assert!(
        a.get("content").is_none() && a.get("data").is_none(),
        "the content does not show up in the response"
    );
    let id = a["id"].as_i64().unwrap_or(-1);
    let msg = r
        .call(
            "POST",
            &format!("/v1/sessions/{s}/messages"),
            Some(json!({"body_md": "file", "client_id": UUID_A, "attachment_ids": [id]})),
        )
        .await;
    assert_eq!(msg.status, 201);
    let att = &msg.json()["meta"]["attachments"];
    assert_eq!(att[0]["name"], evil);
    assert_eq!(att[0]["size"], 16);
    let rows = r.log_rows("SELECT meta_json FROM messages WHERE meta_json IS NOT NULL");
    assert!(
        rows.iter().all(|x| !x[0]
            .clone()
            .unwrap_or_default()
            .contains("secret-file-body")),
        "the content is not in the DB"
    );
    let dl = r.call("GET", &format!("/v1/attachments/{id}"), None).await;
    assert_eq!(dl.status, 200);
    assert_eq!(dl.body, b"secret-file-body");
    assert_eq!(
        r.call("GET", "/v1/attachments/9999", None).await.status,
        404
    );
    let no_file = send(
        r.d.socket_path(),
        Req::new("POST", &format!("/v1/sessions/{s}/attachments"))
            .token(&r.admin)
            .raw("multipart/form-data; boundary=zz", b"--zz--\r\n".to_vec()),
    )
    .await;
    assert_eq!(no_file.status, 422, "without the file field");
    let unknown = r
        .call(
            "POST",
            &format!("/v1/sessions/{s}/messages"),
            Some(json!({"body_md": "x", "client_id": UUID_B, "attachment_ids": [424242]})),
        )
        .await;
    assert_eq!(unknown.status, 422, "unknown attachment");
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_and_path_change_via_patch() {
    let r = rig().await;
    let s = r.session().await;
    let personas = r.get("/v1/personas").await;
    let work = personas
        .as_array()
        .and_then(|a| a.iter().find(|p| p["name"] == "Work"))
        .map_or(-1, |p| p["id"].as_i64().unwrap_or(-1));
    let patched = r
        .call(
            "PATCH",
            &format!("/v1/sessions/{s}"),
            Some(json!({"persona_id": work, "path": "/home/user/code/x"})),
        )
        .await;
    assert_eq!(patched.status, 200);
    let j = patched.json();
    assert_eq!(j["persona_id"], work);
    assert_eq!(j["effective_persona_id"], work);
    assert_eq!(j["path"], "/home/user/code/x");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_removes_messages_keeps_log() {
    let r = rig().await;
    let s = r.session().await;
    r.say(s, "/ask delete file", UUID_A).await;
    for _ in 0..500 {
        if !r.d.pending_requests().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        r.d.pending_requests().len(),
        1,
        "permission request created"
    );
    let del = r.call("DELETE", &format!("/v1/sessions/{s}"), None).await;
    assert_eq!(del.status, 204);
    let left = r.log_rows(&format!(
        "SELECT count(*) FROM messages WHERE session_id = {s}"
    ));
    assert_eq!(left[0][0].as_deref(), Some("0"), "messages went by cascade");
    let log = r.log_rows("SELECT count(*), sum(session_id IS NULL), sum(args_json IS NULL) FROM actions_log WHERE action = 'delete'");
    assert_ne!(log[0][0].as_deref(), Some("0"), "the log stayed");
    assert_eq!(log[0][0], log[0][1], "session_id = NULL");
    assert_eq!(log[0][0], log[0][2], "args_json = NULL");
}
