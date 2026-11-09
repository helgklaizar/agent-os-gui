//! `/personas`: personas (create, edit and delete are admin; read is any token).

use std::sync::Arc;

use agentd_core::tree::parse_persona;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};

use super::{created, invalid, map_err, no_content, ok, parse_id, store};
use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;

type St = State<Arc<ApiState>>;

async fn list(State(state): St) -> Result<Response, ApiError> {
    Ok(ok(store(&state)?.list_personas().await.map_err(map_err)?))
}

async fn create(State(state): St, body: Body) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let input = parse_persona(&read_json(body).await?, true).map_err(invalid)?;
    Ok(created(store.create_persona(input).await.map_err(map_err)?))
}

async fn patch(State(state): St, Path(id): Path<String>, body: Body) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let input = parse_persona(&read_json(body).await?, false).map_err(invalid)?;
    Ok(ok(store.patch_persona(id, input).await.map_err(map_err)?))
}

async fn delete(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    store.delete_persona(id).await.map_err(map_err)?;
    Ok(no_content())
}

pub(crate) fn routes() -> (MethodRouter<Arc<ApiState>>, MethodRouter<Arc<ApiState>>) {
    (
        get(list).post(create),
        axum::routing::patch(patch).delete(delete),
    )
}
