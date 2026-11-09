//! Tasks and schedule (docs/reference/api.md 5.8): `/sessions/{id}/tasks`, `/tasks`, `/tasks/{id}`, `/tasks/{id}/runs`,
//! `/tasks/{id}/run`. Handlers are thin: validation and the `next_run` calculation are in `agentd_core::Tasks`.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{
    FieldError, RUNS_DEFAULT_LIMIT, RUNS_MAX_LIMIT, TaskError, Tasks, parse_task_filter,
};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get, post};
use serde_json::Value;

use crate::body::read_bytes_limited;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{created, invalid, no_content, ok, parse_id};

type St = State<Arc<ApiState>>;

fn tasks(state: &ApiState) -> Result<Tasks, ApiError> {
    state
        .services()
        .and_then(|s| s.tasks.clone())
        .ok_or_else(|| ApiError::core_down("Scheduler unavailable"))
}

fn map_err(e: TaskError) -> ApiError {
    match e {
        TaskError::NotFound => ApiError::new(404, "not_found", "Task not found"),
        TaskError::SessionNotFound => ApiError::new(404, "not_found", "Session not found"),
        TaskError::Invalid(f) => invalid(f),
        TaskError::NoSession => ApiError::conflict("no_session"),
        TaskError::Unavailable => ApiError::core_down("Scheduler unavailable"),
    }
}

async fn json_body(body: Body) -> Result<Value, ApiError> {
    let bytes = read_bytes_limited(body, crate::body::BODY_LIMIT).await?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Request body is not JSON"))
}

fn query_pairs(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (p.to_string(), String::new()),
        })
        .collect()
}

async fn session_list(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (t, sid) = (tasks(&state)?, parse_id(&id)?);
    let list = t.of_session(sid).await.map_err(map_err)?;
    Ok(ok(Value::Array(
        list.iter().map(agentd_core::Task::to_json).collect(),
    )))
}

async fn session_create(
    State(state): St,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let (t, sid) = (tasks(&state)?, parse_id(&id)?);
    let task = t
        .create(sid, &json_body(body).await?)
        .await
        .map_err(map_err)?;
    Ok(created(task.to_json()))
}

async fn all(State(state): St, RawQuery(raw): RawQuery) -> Result<Response, ApiError> {
    let t = tasks(&state)?;
    let filter = parse_task_filter(&query_pairs(raw)).map_err(invalid)?;
    let list = t.list(filter).await.map_err(map_err)?;
    Ok(ok(Value::Array(
        list.iter().map(agentd_core::Task::to_json).collect(),
    )))
}

async fn patch(State(state): St, Path(id): Path<String>, body: Body) -> Result<Response, ApiError> {
    let (t, id) = (tasks(&state)?, parse_id(&id)?);
    let task = t.edit(id, &json_body(body).await?).await.map_err(map_err)?;
    Ok(ok(task.to_json()))
}

async fn delete(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (t, id) = (tasks(&state)?, parse_id(&id)?);
    t.delete(id).await.map_err(map_err)?;
    Ok(no_content())
}

async fn runs(
    State(state): St,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let (t, id) = (tasks(&state)?, parse_id(&id)?);
    let q = query_pairs(raw);
    let mut errs = Vec::new();
    let before = match q.get("before") {
        None => None,
        Some(v) => match v.parse::<i64>() {
            Ok(n) if n > 0 => Some(n),
            _ => {
                errs.push(FieldError::new("before", "positive integer"));
                None
            }
        },
    };
    let limit = match q.get("limit") {
        None => RUNS_DEFAULT_LIMIT,
        Some(v) => match v.parse::<i64>() {
            Ok(n) if (1..=RUNS_MAX_LIMIT).contains(&n) => n,
            _ => {
                errs.push(FieldError::new("limit", "integer from 1 to 200"));
                RUNS_DEFAULT_LIMIT
            }
        },
    };
    if !errs.is_empty() {
        return Err(invalid(errs));
    }
    let list = t.runs(id, before, limit).await.map_err(map_err)?;
    Ok(ok(Value::Array(
        list.iter().map(agentd_core::TaskRun::to_json).collect(),
    )))
}

async fn run(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (t, id) = (tasks(&state)?, parse_id(&id)?);
    let run = t.run_now(id).await.map_err(map_err)?;
    Ok((StatusCode::ACCEPTED, Json(run.to_json())).into_response())
}

pub(crate) struct Routes {
    pub session: MethodRouter<Arc<ApiState>>,
    pub all: MethodRouter<Arc<ApiState>>,
    pub one: MethodRouter<Arc<ApiState>>,
    pub runs: MethodRouter<Arc<ApiState>>,
    pub run: MethodRouter<Arc<ApiState>>,
}

pub(crate) fn routes() -> Routes {
    Routes {
        session: get(session_list).post(session_create),
        all: get(all),
        one: axum::routing::patch(patch).delete(delete),
        runs: get(runs),
        run: post(run),
    }
}
