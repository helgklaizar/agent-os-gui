//! A5-12: `contract_a5_tasks_routes` checks task and trigger route replies against the `openapi-draft.yaml` schemas.

mod common;
mod openapi;

use common::{Env, deleg, finished_runs, mk_task, sched};
use openapi::{check, declared, schemas};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn contract_a5_tasks_routes() {
    let all = schemas();
    for name in ["Task", "TaskRun", "Trigger"] {
        assert!(!all[name].props.is_empty(), "schema {name} was not parsed");
    }
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut seen: Vec<(&str, &str, u16)> = Vec::new();
    let mut rec = |m: &'static str, p: &'static str, st: u16| seen.push((m, p, st));

    let t = mk_task(&e, s, sched("#scn:chunks")).await;
    rec("post", "/sessions/{id}/tasks", 201);
    check("Task", &t, &all);
    let id = t["id"].as_i64().unwrap();
    let d = mk_task(&e, s, deleg("#scn:chunks")).await;
    let did = d["id"].as_i64().unwrap();

    let (st, list) = e.read.get(&format!("/sessions/{s}/tasks")).await;
    rec("get", "/sessions/{id}/tasks", st);
    assert_eq!(st, 200);
    list.as_array()
        .unwrap()
        .iter()
        .for_each(|x| check("Task", x, &all));
    let (st, list) = e.read.get("/tasks").await;
    rec("get", "/tasks", st);
    list.as_array()
        .unwrap()
        .iter()
        .for_each(|x| check("Task", x, &all));
    let (st, bad) = e.read.get("/tasks?state=zzz").await;
    rec("get", "/tasks", st);
    assert_eq!(st, 422, "{bad}");
    let (st, p) = e
        .chat
        .patch(&format!("/tasks/{id}"), json!({"state": "paused"}))
        .await;
    rec("patch", "/tasks/{id}", st);
    check("Task", &p, &all);
    let (st, run) = e.chat.post(&format!("/tasks/{did}/run"), json!({})).await;
    rec("post", "/tasks/{id}/run", st);
    assert_eq!(st, 202);
    check("TaskRun", &run, &all);
    finished_runs(&e, did, 1).await;
    let (st, runs) = e.read.get(&format!("/tasks/{did}/runs")).await;
    rec("get", "/tasks/{id}/runs", st);
    runs.as_array()
        .unwrap()
        .iter()
        .for_each(|x| check("TaskRun", x, &all));
    let (st, _) = e.read.get("/tasks/999/runs").await;
    rec("get", "/tasks/{id}/runs", st);
    assert_eq!(st, 404);
    let (st, _) = e.chat.delete(&format!("/tasks/{id}")).await;
    rec("delete", "/tasks/{id}", st);
    assert_eq!(st, 204);

    let (st, tr) = e
        .admin
        .post(
            "/triggers",
            json!({"source": "tg", "match": {"from": "@a"}, "session_id": s}),
        )
        .await;
    rec("post", "/triggers", st);
    assert_eq!(st, 201, "{tr}");
    check("Trigger", &tr, &all);
    let tid = tr["id"].as_i64().unwrap();
    let (st, list) = e.read.get("/triggers").await;
    rec("get", "/triggers", st);
    list.as_array()
        .unwrap()
        .iter()
        .for_each(|x| check("Trigger", x, &all));
    let (st, tr) = e
        .admin
        .patch(&format!("/triggers/{tid}"), json!({"enabled": false}))
        .await;
    rec("patch", "/triggers/{id}", st);
    check("Trigger", &tr, &all);
    let (st, _) = e.admin.delete(&format!("/triggers/{tid}")).await;
    rec("delete", "/triggers/{id}", st);
    assert_eq!(st, 204);

    assert!(seen.len() >= 13, "{} calls", seen.len());
    for (method, path, st) in &seen {
        let ok = declared(path, method);
        assert!(
            !ok.is_empty(),
            "{method} {path}: operation is not in the specification"
        );
        assert!(
            ok.contains(st) || matches!(st, 403 | 404 | 409 | 422 | 428),
            "{method} {path} → {st}, declared {ok:?}"
        );
    }
}
