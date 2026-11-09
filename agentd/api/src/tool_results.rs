//! `POST /sessions/{id}/tool-results` (docs/reference/api.md 5.2.1): result of a command sent to the browser. Accepted only from the token
//! that received the `browser.command` event with this `request_id`; repeating the same result is idempotent.

use std::sync::Arc;

use agentd_core::{BrowserBridge, FieldError, Principal, SubmitError, parse_tool_result};
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};
use serde_json::json;

use crate::body::read_json_limited;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, parse_id};

/// Request body is at most 1 MiB (output up to 256 KiB, the rest is truncated by the core).
const BODY_LIMIT: usize = 1024 * 1024;

fn bridge(state: &ApiState) -> Result<BrowserBridge, ApiError> {
    state
        .services()
        .and_then(|s| s.browser.clone())
        .ok_or_else(|| ApiError::core_down("Browser command bridge unavailable"))
}

fn map_submit(e: SubmitError) -> ApiError {
    match e {
        SubmitError::Unknown => ApiError::not_found(),
        SubmitError::Foreign => ApiError::permission_denied("not_command_owner"),
        SubmitError::Closed => ApiError::conflict("request_closed"),
        SubmitError::Conflict => ApiError::conflict("result_exists"),
        SubmitError::KindMismatch => invalid(vec![FieldError::new(
            "kind",
            "does not match the command kind",
        )]),
    }
}

async fn submit(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let (bridge, session_id) = (bridge(&state)?, parse_id(&id)?);
    let res = parse_tool_result(&read_json_limited(body, BODY_LIMIT).await?).map_err(invalid)?;
    let rid = res.request_id.clone();
    bridge
        .submit(session_id, who.token_id, res)
        .map_err(map_submit)?;
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(json!({"request_id": rid, "status": "accepted"})),
    )
        .into_response())
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    post(submit)
}
