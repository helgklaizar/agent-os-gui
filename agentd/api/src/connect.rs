//! `POST /accounts/{id}/connect` and `DELETE /accounts/{id}/connect` (docs/reference/api.md 5.6): connecting a channel account.

use std::sync::Arc;

use agentd_core::{ChannelError, Channels};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};

use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{no_content, parse_id};

fn channels(state: &ApiState) -> Result<Channels, ApiError> {
    state
        .services()
        .and_then(|s| s.channels.clone())
        .ok_or_else(|| ApiError::core_down("Connections unavailable"))
}

fn map_err(e: ChannelError) -> ApiError {
    match e {
        ChannelError::NotFound => ApiError::new(404, "not_found", "Account not found"),
        ChannelError::NotChannel => ApiError::conflict("not_a_channel"),
        ChannelError::Unavailable => ApiError::core_down("Connections unavailable"),
    }
}

/// The request body is unused; the response is `202 ConnectFlow`.
async fn start(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (ch, id) = (channels(&state)?, parse_id(&id)?);
    let flow = ch.connect(id).await.map_err(map_err)?;
    Ok((StatusCode::ACCEPTED, Json(flow)).into_response())
}

async fn stop(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (ch, id) = (channels(&state)?, parse_id(&id)?);
    ch.disconnect(id).await.map_err(map_err)?;
    Ok(no_content())
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    post(start).delete(stop)
}
