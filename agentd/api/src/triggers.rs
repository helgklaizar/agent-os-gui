//! `/triggers` (docs/reference/api.md 5.8): list (read), create, edit and delete (admin). Body validation and firing are in
//! `agentd_core::Triggers`.

use std::sync::Arc;

use agentd_core::{TriggerError, Triggers};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::Value;

use crate::body::read_bytes_limited;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{created, invalid, no_content, ok, parse_id};

type St = State<Arc<ApiState>>;

fn triggers(state: &ApiState) -> Result<Triggers, ApiError> {
    state
        .services()
        .and_then(|s| s.triggers.clone())
        .ok_or_else(|| ApiError::core_down("Triggers unavailable"))
}

fn map_err(e: TriggerError) -> ApiError {
    match e {
        TriggerError::NotFound => ApiError::new(404, "not_found", "Trigger not found"),
        TriggerError::SessionNotFound => ApiError::new(404, "not_found", "Session not found"),
        TriggerError::TaskNotFound => invalid(vec![agentd_core::FieldError::new(
            "task_id",
            "task not found",
        )]),
        TriggerError::Invalid(f) => invalid(f),
        TriggerError::Unavailable => ApiError::core_down("Triggers unavailable"),
    }
}

async fn json_body(body: Body) -> Result<Value, ApiError> {
    let bytes = read_bytes_limited(body, crate::body::BODY_LIMIT).await?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Request body is not JSON"))
}

async fn list(State(state): St) -> Result<Response, ApiError> {
    let list = triggers(&state)?.list().await.map_err(map_err)?;
    Ok(ok(Value::Array(
        list.iter().map(agentd_core::Trigger::to_json).collect(),
    )))
}

async fn create(State(state): St, body: Body) -> Result<Response, ApiError> {
    let t = triggers(&state)?;
    let made = t.create(&json_body(body).await?).await.map_err(map_err)?;
    Ok(created(made.to_json()))
}

async fn patch(State(state): St, Path(id): Path<String>, body: Body) -> Result<Response, ApiError> {
    let (t, id) = (triggers(&state)?, parse_id(&id)?);
    let done = t.edit(id, &json_body(body).await?).await.map_err(map_err)?;
    Ok(ok(done.to_json()))
}

async fn delete(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (t, id) = (triggers(&state)?, parse_id(&id)?);
    t.delete(id).await.map_err(map_err)?;
    Ok(no_content())
}

pub(crate) fn routes() -> (MethodRouter<Arc<ApiState>>, MethodRouter<Arc<ApiState>>) {
    (
        get(list).post(create),
        axum::routing::patch(patch).delete(delete),
    )
}
