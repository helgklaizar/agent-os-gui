//! Boundaries of bookmark and tool-results checks in `fake-agentd`, found by mutations: spaces and control characters in the address,
//! list limit, `rev` growth, `request_id` length, closing commands on `stop`.

mod chat_common;
mod events_common;

use events_common::*;
use serde_json::{Value, json};

async fn put(r: &Rig, path: &str, etag: &str, body: Value) -> Resp {
    send(
        r.d.socket_path(),
        Req::new("PUT", path)
            .token(&r.admin)
            .header("If-Match", etag)
            .json(body),
    )
    .await
}

fn link(n: usize) -> Value {
    json!({"title": format!("t{n}"), "url": format!("https://example.org/{n}")})
}

#[tokio::test(flavor = "multi_thread")]
async fn bookmarks_reject_whitespace_control_and_overlong_urls_and_cap_the_list() {
    let r = bare().await;
    let s = r.session().await;
    let path = format!("/v1/sessions/{s}/bookmarks");
    let etag = r
        .call("GET", &path, None)
        .await
        .header("etag")
        .unwrap_or("")
        .to_string();
    for bad in [
        "https://a.b/ x",
        "https://a.b/\tx",
        "https://a.b/\u{7}",
        "https://a.b/\u{1b}[31m",
    ] {
        let resp = put(&r, &path, &etag, json!([{"title": "t", "url": bad}])).await;
        assert_eq!(resp.status, 422, "{bad:?}");
    }
    let long_ok = format!("https://a.b/{}", "x".repeat(2048 - "https://a.b/".len()));
    assert_eq!(long_ok.len(), 2048);
    let ok = put(&r, &path, &etag, json!([{"title": "t", "url": long_ok}])).await;
    assert_eq!(ok.status, 200, "exactly 2048 characters is allowed");
    let etag2 = ok.header("etag").unwrap_or("").to_string();
    let too_long = format!("{long_ok}x");
    let bad = put(&r, &path, &etag2, json!([{"title": "t", "url": too_long}])).await;
    assert_eq!(bad.status, 422, "2049 characters - refusal");
    let upper = put(
        &r,
        &path,
        &etag2,
        json!([{"title": "t", "url": "HTTPS://A.B/"}]),
    )
    .await;
    assert_eq!(upper.status, 200, "the scheme is case-insensitive");
    let etag3 = upper.header("etag").unwrap_or("").to_string();

    let many: Vec<Value> = (0..500).map(link).collect();
    let full = put(&r, &path, &etag3, Value::Array(many)).await;
    assert_eq!(full.status, 200, "500 links are allowed");
    let etag4 = full.header("etag").unwrap_or("").to_string();
    let over: Vec<Value> = (0..501).map(link).collect();
    assert_eq!(
        put(&r, &path, &etag4, Value::Array(over)).await.status,
        422,
        "501 links - refusal"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bookmarks_rev_grows_by_one_per_write() {
    let r = bare().await;
    let s = r.session().await;
    let path = format!("/v1/sessions/{s}/bookmarks");
    let first = r.call("GET", &path, None).await;
    assert_eq!(first.header("etag"), Some("\"0\""), "empty list - rev 0");
    let one = put(&r, &path, "\"0\"", json!([link(1)])).await;
    assert_eq!(one.header("etag"), Some("\"1\""));
    let two = put(&r, &path, "\"1\"", json!([link(1), link(2)])).await;
    assert_eq!(two.header("etag"), Some("\"2\""));
    assert_eq!(
        r.call("GET", &path, None).await.header("etag"),
        Some("\"2\"")
    );
}

async fn result(r: &Rig, s: i64, body: Value) -> Resp {
    r.call(
        "POST",
        &format!("/v1/sessions/{s}/tool-results"),
        Some(body),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_request_id_length_and_status_are_checked() {
    let r = bare().await;
    let s = r.session().await;
    let rid = r.d.browser_command(s, "https://example.org/");
    let ok = json!({"request_id": rid, "kind": "browser", "status": "ok"});
    for bad in [
        json!({"request_id": "", "kind": "browser", "status": "ok"}),
        json!({"request_id": "r".repeat(201), "kind": "browser", "status": "ok"}),
        json!({"request_id": rid, "kind": "browser", "status": "weird"}),
        json!({"request_id": rid, "kind": "browser"}),
        json!({"request_id": rid, "kind": "browser", "status": "ok", "extra": 1}),
        json!({"kind": "browser", "status": "ok"}),
    ] {
        assert_eq!(result(&r, s, bad.clone()).await.status, 422, "{bad}");
    }
    let unknown_200 = json!({"request_id": "r".repeat(200), "kind": "browser", "status": "ok"});
    assert_eq!(
        result(&r, s, unknown_200).await.status,
        404,
        "200 characters is a valid length, but there is no such request"
    );
    assert_eq!(result(&r, s, ok).await.status, 202);
    for st in ["ok", "error", "timeout", "cancelled", "denied"] {
        let id = r.d.browser_command(s, "https://example.org/");
        let resp = result(
            &r,
            s,
            json!({"request_id": id, "kind": "browser", "status": st}),
        )
        .await;
        assert_eq!(resp.status, 202, "{st}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_closed_by_stop_but_answered_ones_stay_idempotent() {
    let r = bare().await;
    let s = r.session().await;
    let other = r.session().await;
    let answered = r.d.browser_command(s, "https://one.example/");
    let open = r.d.browser_command(s, "https://two.example/");
    let foreign = r.d.browser_command(other, "https://three.example/");
    let done = json!({"request_id": answered, "kind": "browser", "status": "ok", "output": "x"});
    assert_eq!(result(&r, s, done.clone()).await.status, 202);
    assert_eq!(
        r.call("POST", &format!("/v1/sessions/{s}/stop"), None)
            .await
            .status,
        204
    );
    let late = result(
        &r,
        s,
        json!({"request_id": open, "kind": "browser", "status": "ok"}),
    )
    .await;
    assert_eq!(late.status, 409, "after stop the open command is closed");
    assert_eq!(late.json()["error"]["details"]["reason"], "request_closed");
    assert_eq!(
        result(&r, s, done).await.status,
        202,
        "an already accepted result repeats idempotently even after stop"
    );
    let theirs = result(
        &r,
        other,
        json!({"request_id": foreign, "kind": "browser", "status": "ok"}),
    )
    .await;
    assert_eq!(
        theirs.status, 202,
        "stop of another session does not close the command"
    );
    let wrong_session = result(
        &r,
        other,
        json!({"request_id": open, "kind": "browser", "status": "ok"}),
    )
    .await;
    assert_eq!(
        wrong_session.status, 404,
        "a request of another session is not visible"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tasks_validation_edges_and_per_session_listing() {
    let r = bare().await;
    let (a, b) = (r.session().await, r.session().await);
    let sched = json!({"kind": "sched", "title": "t", "spec": "s", "cron": "0 9 * * *"});
    let deleg = json!({"kind": "deleg", "title": "d", "spec": "s"});
    let post = |s: i64, body: Value| {
        let r = &r;
        async move {
            r.call("POST", &format!("/v1/sessions/{s}/tasks"), Some(body))
                .await
                .status
        }
    };
    assert_eq!(post(a, sched.clone()).await, 201);
    assert_eq!(post(b, sched.clone()).await, 201);
    assert_eq!(post(a, deleg).await, 201);
    for bad in [
        json!({"kind": "sched", "title": "t", "spec": "s", "cron": "* *"}),
        json!({"kind": "sched", "title": "t", "spec": "s", "cron": null}),
        json!({"kind": "deleg", "title": "t", "spec": "s", "cron": "0 9 * * *"}),
    ] {
        assert_eq!(post(a, bad.clone()).await, 422, "{bad}");
    }
    let list = |s: i64| {
        let r = &r;
        async move { r.get(&format!("/v1/sessions/{s}/tasks")).await }
    };
    assert_eq!(
        (
            list(a).await.as_array().map_or(0, Vec::len),
            list(b).await.as_array().map_or(0, Vec::len)
        ),
        (2, 1),
        "only the session's own list"
    );
    let tasks = list(a).await;
    let id = tasks[0]["id"].as_i64().unwrap_or(-1);
    let deleg_id = tasks[1]["id"].as_i64().unwrap_or(-1);
    let cleared = r
        .call(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"cron": null})),
        )
        .await;
    assert_eq!(cleared.status, 200);
    assert_eq!(
        cleared.json()["cron"],
        Value::Null,
        "cron: null removes the schedule"
    );
    let bad = r
        .call(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"cron": "x"})),
        )
        .await;
    assert_eq!(bad.status, 422);
    r.call("POST", &format!("/v1/tasks/{deleg_id}/run"), None)
        .await;
    r.call("POST", &format!("/v1/tasks/{id}/run"), None).await;
    let after = list(a).await;
    assert_eq!(
        after[1]["state"], "done",
        "the delegated one finished after the run"
    );
    assert_eq!(
        after[0]["state"], "active",
        "the scheduled one stays active"
    );
}
