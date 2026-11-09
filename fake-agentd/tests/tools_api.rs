//! Session tasks, bookmarks and quick links, tool results (api.md sections 5.2.1, 5.8; openapi-draft.yaml).

mod chat_common;
mod events_common;

use events_common::*;
use serde_json::{Value, json};

fn task_body(kind: &str) -> Value {
    if kind == "sched" {
        json!({"kind": "sched", "title": "Morning report", "spec": "collect metrics", "cron": "0 9 * * *"})
    } else {
        json!({"kind": "deleg", "title": "Sort the mail", "spec": "reply to incoming"})
    }
}

async fn create(r: &Rig, session: i64, body: Value) -> Value {
    let resp = r
        .call("POST", &format!("/v1/sessions/{session}/tasks"), Some(body))
        .await;
    assert_eq!(
        resp.status,
        201,
        "creation: {}",
        String::from_utf8_lossy(&resp.body)
    );
    resp.json()
}

#[tokio::test(flavor = "multi_thread")]
async fn task_create_run_emits_event() {
    let r = bare().await;
    let s = r.session().await;
    let events = watch(&r.d, &r.admin, "events").await;
    // the subscription is accepted when the snapshot arrived
    assert!(
        events
            .until(3, |w| (!w.all().is_empty()).then_some(()))
            .await
            .is_some()
    );
    let task = create(&r, s, task_body("sched")).await;
    let id = task["id"].as_i64().unwrap_or(-1);
    assert!(id > 0);
    assert_eq!(task["kind"], "sched");
    assert_eq!(task["state"], "active");
    assert_eq!(task["tz"], "Asia/Jerusalem");
    assert_eq!(task["session_id"], s);
    let run = r.call("POST", &format!("/v1/tasks/{id}/run"), None).await;
    assert_eq!(run.status, 202, "{}", String::from_utf8_lossy(&run.body));
    let run = run.json();
    assert_eq!(run["task_id"], id);
    assert!(
        ["running", "ok"].contains(&run["status"].as_str().unwrap_or("")),
        "{run}"
    );
    let rows = r.log_rows(&format!(
        "SELECT status, task_id FROM task_runs WHERE task_id = {id}"
    ));
    assert_eq!(rows.len(), 1, "run recorded in task_runs");
    let ev = events
        .until(3, |w| {
            w.all()
                .into_iter()
                .find(|f| {
                    f["type"] == "event"
                        && f["data"]["kind"] == "task"
                        && f["data"]["sub"] == "sched"
                })
                .map(|f| f["data"].clone())
        })
        .await;
    let ev = ev.unwrap_or(Value::Null);
    assert_eq!(
        ev["session_id"], s,
        "task/sched event is bound to the session: {ev}"
    );
    let list = r.get("/v1/events?kind=task").await;
    let subs: Vec<&str> = list["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["sub"].as_str()).collect())
        .unwrap_or_default();
    assert!(subs.contains(&"sched"), "the feed has task/sched: {subs:?}");
    assert!(
        subs.contains(&"done"),
        "and task/done on completion: {subs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_runs_show_last_runs() {
    let r = bare().await;
    let s = r.session().await;
    let id = create(&r, s, task_body("deleg")).await["id"]
        .as_i64()
        .unwrap_or(-1);
    for _ in 0..3 {
        r.call("POST", &format!("/v1/tasks/{id}/run"), None).await;
    }
    let runs = r.get(&format!("/v1/tasks/{id}/runs?limit=2")).await;
    let runs = runs.as_array().cloned().unwrap_or_default();
    assert_eq!(runs.len(), 2, "limit is respected");
    assert!(
        runs[0]["id"].as_i64() > runs[1]["id"].as_i64(),
        "newest to oldest"
    );
    assert_eq!(runs[0]["status"], "ok");
    let bad = r
        .call("GET", &format!("/v1/tasks/{id}/runs?limit=0"), None)
        .await;
    assert_eq!(bad.status, 422);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_validation_patch_delete_and_filters() {
    let r = bare().await;
    let s = r.session().await;
    let path = format!("/v1/sessions/{s}/tasks");
    let no_cron = r
        .call(
            "POST",
            &path,
            Some(json!({"kind": "sched", "title": "t", "spec": "s"})),
        )
        .await;
    assert_eq!(no_cron.status, 422, "sched without cron");
    let bad_kind = r
        .call(
            "POST",
            &path,
            Some(json!({"kind": "x", "title": "t", "spec": "s"})),
        )
        .await;
    assert_eq!(bad_kind.status, 422);
    let empty = r
        .call(
            "POST",
            &path,
            Some(json!({"kind": "deleg", "title": "", "spec": "s"})),
        )
        .await;
    assert_eq!(empty.status, 422, "empty title");
    let ghost = r
        .call("POST", "/v1/sessions/9999/tasks", Some(task_body("deleg")))
        .await;
    assert_eq!(ghost.status, 404);

    let id = create(&r, s, task_body("sched")).await["id"]
        .as_i64()
        .unwrap_or(-1);
    let other = create(&r, s, task_body("deleg")).await["id"]
        .as_i64()
        .unwrap_or(-1);
    let paused = r
        .call(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"state": "paused"})),
        )
        .await;
    assert_eq!(paused.status, 200);
    assert_eq!(paused.json()["state"], "paused");
    assert_eq!(
        r.get(&format!("/v1/tasks?state=paused&session_id={s}"))
            .await
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        r.get("/v1/tasks?kind=deleg").await.as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        r.get(&format!("/v1/sessions/{s}/tasks"))
            .await
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let bad_state = r
        .call(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"state": "gone"})),
        )
        .await;
    assert_eq!(bad_state.status, 422);
    let del = r.call("DELETE", &format!("/v1/tasks/{other}"), None).await;
    assert_eq!(del.status, 204);
    let again = r.call("DELETE", &format!("/v1/tasks/{other}"), None).await;
    assert_eq!(again.status, 404);
    let read = r.token(fake_agentd::Scope::Read, fake_agentd::Kind::Test);
    let denied = r
        .call_as(&read, "POST", &path, Some(task_body("deleg")))
        .await;
    assert_eq!(denied.status, 403, "creating tasks needs scope chat");
}

#[tokio::test(flavor = "multi_thread")]
async fn bookmarks_if_match_roundtrip() {
    let r = bare().await;
    let s = r.session().await;
    let path = format!("/v1/sessions/{s}/bookmarks");
    let first = r.call("GET", &path, None).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.json(), json!([]));
    let etag = first.header("etag").unwrap_or("").to_string();
    assert!(!etag.is_empty(), "ETag exists even on an empty list");
    let list = json!([{"title": "HackerNews", "url": "https://news.ycombinator.com/"}]);
    let no_header = r.call("PUT", &path, Some(list.clone())).await;
    assert_eq!(no_header.status, 428, "without If-Match");
    let stale = send(
        r.d.socket_path(),
        Req::new("PUT", &path)
            .token(&r.admin)
            .header("If-Match", "\"999\"")
            .json(list.clone()),
    )
    .await;
    assert_eq!(stale.status, 412);
    let ok = send(
        r.d.socket_path(),
        Req::new("PUT", &path)
            .token(&r.admin)
            .header("If-Match", &etag)
            .json(list.clone()),
    )
    .await;
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(ok.json(), list);
    assert_ne!(ok.header("etag").unwrap_or(""), etag, "ETag changed");
    assert_eq!(r.get(&path).await, list);
    let rows = r.log_rows(&format!(
        "SELECT value_json FROM settings WHERE key = 'browser.bookmarks.{s}'"
    ));
    assert_eq!(rows.len(), 1, "stored in settings, no separate table");
    let other = r.session().await;
    assert_eq!(
        r.get(&format!("/v1/sessions/{other}/bookmarks")).await,
        json!([]),
        "bookmarks are not mixed between sessions"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bookmarks_reject_bad_links_and_die_with_session() {
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
        json!([{"title": "x", "url": "javascript:alert(1)"}]),
        json!([{"title": "", "url": "https://a.b/"}]),
        json!([{"url": "https://a.b/"}]),
        json!({"title": "x"}),
    ] {
        let resp = send(
            r.d.socket_path(),
            Req::new("PUT", &path)
                .token(&r.admin)
                .header("If-Match", &etag)
                .json(bad.clone()),
        )
        .await;
        assert_eq!(resp.status, 422, "{bad}");
    }
    let good = json!([{"title": "HackerNews", "url": "https://news.ycombinator.com/"}]);
    let saved = send(
        r.d.socket_path(),
        Req::new("PUT", &path)
            .token(&r.admin)
            .header("If-Match", &etag)
            .json(good),
    )
    .await;
    assert_eq!(saved.status, 200);
    let del = r.call("DELETE", &format!("/v1/sessions/{s}"), None).await;
    assert_eq!(del.status, 204);
    assert!(
        r.log_rows(&format!(
            "SELECT 1 FROM settings WHERE key = 'browser.bookmarks.{s}'"
        ))
        .is_empty(),
        "bookmarks of the deleted session are cleaned up"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quicklinks_roundtrip_with_if_match() {
    let r = bare().await;
    let first = r.call("GET", "/v1/browser/quicklinks", None).await;
    assert_eq!(first.status, 200);
    let links = first.json();
    assert_eq!(
        links.as_array().map(Vec::len),
        Some(5),
        "Google, YouTube, GitHub, Wikipedia, HackerNews"
    );
    let etag = first.header("etag").unwrap_or("").to_string();
    let mut next = links.as_array().cloned().unwrap_or_default();
    next.push(json!({"title": "Example", "url": "https://example.org/"}));
    let ok = send(
        r.d.socket_path(),
        Req::new("PUT", "/v1/browser/quicklinks")
            .token(&r.admin)
            .header("If-Match", &etag)
            .json(Value::Array(next.clone())),
    )
    .await;
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(
        r.get("/v1/browser/quicklinks").await,
        Value::Array(next.clone())
    );
    let stale = send(
        r.d.socket_path(),
        Req::new("PUT", "/v1/browser/quicklinks")
            .token(&r.admin)
            .header("If-Match", &etag)
            .json(Value::Array(next)),
    )
    .await;
    assert_eq!(stale.status, 412, "stale ETag after a write");
}

async fn post_result(r: &Rig, s: i64, body: Value) -> Resp {
    r.call(
        "POST",
        &format!("/v1/sessions/{s}/tool-results"),
        Some(body),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_browser_roundtrip() {
    let r = bare().await;
    let s = r.session().await;
    let watcher = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    assert!(
        watcher
            .until(3, |w| (!w.all().is_empty()).then_some(()))
            .await
            .is_some()
    );
    let rid = r.d.browser_command(s, "https://example.org/");
    assert!(!rid.is_empty(), "request_id issued");
    let frame = watcher
        .event("browser.command", 3)
        .await
        .unwrap_or(Value::Null);
    assert_eq!(frame["request_id"], rid.as_str());
    assert_eq!(frame["url"], "https://example.org/");
    let body = json!({"request_id": rid, "kind": "browser", "status": "ok", "output": "<html></html>",
        "url": "https://example.org/", "http_status": 200});
    let first = post_result(&r, s, body.clone()).await;
    assert_eq!(
        first.status,
        202,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(
        first.json(),
        json!({"request_id": rid, "status": "accepted"})
    );
    let again = post_result(&r, s, body.clone()).await;
    assert_eq!(
        again.status, 202,
        "a repeat of the same result is idempotent"
    );
    let mut other = body.clone();
    other["output"] = json!("other");
    let conflict = post_result(&r, s, other).await;
    assert_eq!(conflict.status, 409);
    assert_eq!(
        conflict.json()["error"]["details"]["reason"],
        "result_exists"
    );
    let ghost = post_result(
        &r,
        s,
        json!({"request_id": "no-such", "kind": "browser", "status": "ok"}),
    )
    .await;
    assert_eq!(ghost.status, 404, "unknown request_id");
    let bad = post_result(
        &r,
        s,
        json!({"request_id": rid, "kind": "terminal", "status": "ok"}),
    )
    .await;
    assert_eq!(bad.status, 422, "kind only browser or user");
    let wrong_session = post_result(&r, s + 1000, body).await;
    assert_eq!(wrong_session.status, 404);
}
