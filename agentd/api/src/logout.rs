//! `POST /logout` (api.md, admin, local only): audit log entry, closing all WS with code `1001`, ending the user session
//! via `loginctl`. Response `202`: the session ends after the response is sent.

use std::sync::Arc;

use agentd_core::Principal;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};
use serde_json::json;

use crate::error::ApiError;
use crate::state::ApiState;

async fn logout(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
) -> Result<Response, ApiError> {
    crate::audit::user_action(
        &state,
        &who,
        "logout",
        "session:self".to_string(),
        json!({}),
    )
    .await;
    state.hub.close_all();
    let closed = match state.services().and_then(|s| s.logout.clone()) {
        Some(c) => c.close().await,
        None => false,
    };
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(json!({"status": "closing", "session_closed": closed})),
    )
        .into_response())
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    post(logout)
}
