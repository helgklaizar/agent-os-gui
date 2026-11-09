//! Helpers for the end-to-end task tests: creation, run history, `task.*` events.
#![allow(dead_code)]

use std::time::Duration;

use serde_json::{Value, json};

use super::Env;

pub async fn mk_task(e: &Env, s: i64, body: Value) -> Value {
    let (st, v) = e.chat.post(&format!("/sessions/{s}/tasks"), body).await;
    assert_eq!(st, 201, "{v}");
    v
}

pub fn sched(spec: &str) -> Value {
    json!({"kind": "sched", "title": "morning", "spec": spec, "cron": "0 8 * * *"})
}

pub fn deleg(spec: &str) -> Value {
    json!({"kind": "deleg", "title": "assignment", "spec": spec})
}

pub async fn runs(e: &Env, task: i64) -> Vec<Value> {
    let (st, v) = e.read.get(&format!("/tasks/{task}/runs")).await;
    assert_eq!(st, 200, "{v}");
    v.as_array().unwrap().clone()
}

/// Waits until the task has at least `n` finished runs (not `running`).
pub async fn finished_runs(e: &Env, task: i64, n: usize) -> Vec<Value> {
    for _ in 0..400 {
        let r = runs(e, task).await;
        let done: Vec<Value> = r.into_iter().filter(|x| x["status"] != "running").collect();
        if done.len() >= n {
            return done;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("runs did not finish");
}

pub async fn task_events(e: &Env, sub: &str) -> Vec<Value> {
    let (_, v) = e.read.get(&format!("/events?kind=task&sub={sub}")).await;
    v["items"].as_array().unwrap().clone()
}

pub fn chrono_now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}
