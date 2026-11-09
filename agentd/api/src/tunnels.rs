//! `/tunnels` (docs/reference/api.md 5.7): list, create, edit and delete (`admin`). No secrets in the fields (checked in the core);
//! changes are written to the owner action log.

use std::sync::Arc;

use agentd_core::{
    Principal, Tunnel, TunnelError, TunnelStore, parse_tunnel_create, parse_tunnel_patch,
};
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::{Value, json};

use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{created, invalid, no_content, ok, parse_id};

type St = State<Arc<ApiState>>;

fn store(state: &ApiState) -> Result<Arc<dyn TunnelStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.tunnels.clone())
        .ok_or_else(|| ApiError::core_down("Tunnels unavailable"))
}

fn map_err(e: TunnelError) -> ApiError {
    match e {
        TunnelError::NotFound => ApiError::new(404, "not_found", "Tunnel not found"),
        TunnelError::Invalid(f) => invalid(f),
        TunnelError::Unavailable => ApiError::core_down("Tunnels unavailable"),
    }
}

/// Tunnel with the current process status.
fn view(state: &ApiState, t: &Tunnel) -> Value {
    match state.services().and_then(|s| s.tunnel_statuses.clone()) {
        Some(st) => t.to_json_with(st.of(t)),
        None => t.to_json(),
    }
}

async fn list(State(state): St) -> Result<Response, ApiError> {
    let items = store(&state)?.list().await.map_err(map_err)?;
    Ok(ok(Value::Array(
        items.iter().map(|t| view(&state, t)).collect(),
    )))
}

async fn create(
    State(state): St,
    Extension(who): Extension<Principal>,
    body: Body,
) -> Result<Response, ApiError> {
    let t = parse_tunnel_create(&read_json(body).await?).map_err(invalid)?;
    let made = store(&state)?.create(t).await.map_err(map_err)?;
    audit(&state, &who, "create", &made).await;
    Ok(created(view(&state, &made)))
}

async fn patch(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let (s, id) = (store(&state)?, parse_id(&id)?);
    let cur = s.get(id).await.map_err(map_err)?;
    let next = parse_tunnel_patch(&cur, &read_json(body).await?).map_err(invalid)?;
    let saved = s.update(next).await.map_err(map_err)?;
    audit(&state, &who, "update", &saved).await;
    Ok(ok(view(&state, &saved)))
}

async fn delete(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (s, id) = (store(&state)?, parse_id(&id)?);
    let cur = s.get(id).await.map_err(map_err)?;
    s.delete(id).await.map_err(map_err)?;
    audit(&state, &who, "delete", &cur).await;
    Ok(no_content())
}

/// Owner log entry: `config_write`, target `tunnel:<id>`; the address and description do not go into the log.
async fn audit(state: &ApiState, who: &Principal, op: &str, t: &Tunnel) {
    crate::audit::user_action(
        state,
        who,
        "config_write",
        format!("tunnel:{}", t.id),
        json!({"op": op, "type": t.kind}),
    )
    .await;
}

pub(crate) fn routes() -> (MethodRouter<Arc<ApiState>>, MethodRouter<Arc<ApiState>>) {
    (
        get(list).post(create),
        axum::routing::patch(patch).delete(delete),
    )
}
