//! Routes and responses of tasks, bookmarks and tool results against `openapi-draft.yaml`
//! (schemas `Task`, `TaskRun`, `ToolResultAck`, `Link`).

use fake_agentd::{tasks_http, tools_http};
use serde_json::{Value, json};

use crate::openapi::Spec;
use crate::rawhttp::{Req, send};
use crate::{assert_clean, mock, status_documented};

#[test]
fn contract_tasks_bookmarks_toolresults_match_openapi() {
    let spec = Spec::load();
    assert_eq!(tasks_http::ROUTES.len(), 7, "task routes in the mock");
    assert_eq!(
        tools_http::ROUTES.len(),
        5,
        "bookmarks, quick links, tool-results"
    );
    for r in tasks_http::ROUTES.iter().chain(tools_http::ROUTES) {
        assert!(
            spec.op(r.method, r.pattern).is_some(),
            "{} {} is in the mock and missing from openapi-draft.yaml",
            r.method,
            r.pattern
        );
        assert_eq!(
            spec.scope(r.method, r.pattern),
            Some(r.scope.as_str()),
            "x-scope {} {}",
            r.method,
            r.pattern
        );
        assert_eq!(spec.local_only(r.method, r.pattern), r.local_only);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_tools_responses_match_schemas() {
    let d = mock().await;
    let spec = Spec::load();
    let (sock, admin) = (d.socket_path(), d.admin_token().to_string());
    let made = send(
        sock,
        Req::new("POST", "/v1/sessions")
            .token(&admin)
            .json(json!({"name": "K"})),
    )
    .await;
    let session = made.json()["id"].as_i64().unwrap_or(-1);

    let task = send(
        sock,
        Req::new("POST", &format!("/v1/sessions/{session}/tasks"))
            .token(&admin)
            .json(json!({"kind": "sched", "title": "t", "spec": "s", "cron": "0 9 * * *"})),
    )
    .await;
    assert_eq!(task.status, 201);
    assert_clean(
        "POST /sessions/{id}/tasks",
        spec.validate(spec.schema("Task"), &task.json(), "Task"),
    );
    assert!(status_documented(
        &spec,
        "POST",
        "/sessions/{id}/tasks",
        201
    ));
    let id = task.json()["id"].as_i64().unwrap_or(-1);

    let list = send(
        sock,
        Req::new("GET", &format!("/v1/sessions/{session}/tasks")).token(&admin),
    )
    .await;
    assert_eq!(list.status, 200);
    for (i, t) in list.json().as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /sessions/{id}/tasks",
            spec.validate(spec.schema("Task"), t, &format!("Task[{i}]")),
        );
    }
    let patched = send(
        sock,
        Req::new("PATCH", &format!("/v1/tasks/{id}"))
            .token(&admin)
            .json(json!({"state": "paused"})),
    )
    .await;
    assert_eq!(patched.status, 200);
    assert_clean(
        "PATCH /tasks/{id}",
        spec.validate(spec.schema("Task"), &patched.json(), "Task"),
    );
    let run = send(
        sock,
        Req::new("POST", &format!("/v1/tasks/{id}/run")).token(&admin),
    )
    .await;
    assert_eq!(run.status, 202);
    assert_clean(
        "POST /tasks/{id}/run",
        spec.validate(spec.schema("TaskRun"), &run.json(), "TaskRun"),
    );
    assert!(status_documented(&spec, "POST", "/tasks/{id}/run", 202));
    let runs = send(
        sock,
        Req::new("GET", &format!("/v1/tasks/{id}/runs")).token(&admin),
    )
    .await;
    assert_eq!(runs.status, 200);
    for (i, r) in runs.json().as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /tasks/{id}/runs",
            spec.validate(spec.schema("TaskRun"), r, &format!("TaskRun[{i}]")),
        );
    }

    for path in [
        format!("/v1/sessions/{session}/bookmarks"),
        "/v1/browser/quicklinks".to_string(),
    ] {
        let got = send(sock, Req::new("GET", &path).token(&admin)).await;
        assert_eq!(got.status, 200, "{path}");
        assert!(got.header("etag").is_some(), "ETag in {path}");
        for (i, l) in got.json().as_array().into_iter().flatten().enumerate() {
            assert_clean(
                &path,
                spec.validate(spec.schema("Link"), l, &format!("Link[{i}]")),
            );
        }
        let no_match = send(sock, Req::new("PUT", &path).token(&admin).json(json!([]))).await;
        assert_eq!(no_match.status, 428);
    }
    for (method, path, status) in [
        ("PUT", "/sessions/{id}/bookmarks", 412),
        ("PUT", "/sessions/{id}/bookmarks", 428),
        ("PUT", "/browser/quicklinks", 412),
        ("PUT", "/browser/quicklinks", 428),
        ("POST", "/sessions/{id}/tool-results", 409),
        ("POST", "/sessions/{id}/tool-results", 404),
        ("POST", "/sessions/{id}/tool-results", 422),
    ] {
        assert!(
            status_documented(&spec, method, path, status),
            "{method} {path}: {status} is not described in openapi-draft.yaml"
        );
    }

    let rid = d.browser_command(session, "https://example.org/");
    let body: Value = json!({"request_id": rid, "kind": "browser", "status": "ok"});
    let ack = send(
        sock,
        Req::new("POST", &format!("/v1/sessions/{session}/tool-results"))
            .token(&admin)
            .json(body),
    )
    .await;
    assert_eq!(ack.status, 202);
    assert_clean(
        "POST /sessions/{id}/tool-results",
        spec.validate(spec.schema("ToolResultAck"), &ack.json(), "ToolResultAck"),
    );
}
