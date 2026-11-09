//! Routes for the session tree, personas and accounts (docs/reference/api.md 5.1, 5.6) and the `sessions`, `connections` topics.
//! Handlers are thin: body parsing is in `agentd_core::tree`, rules and writes are in `TreeStore`.

mod accounts;
mod folders;
mod personas;
mod sessions;

use std::sync::Arc;

use agentd_core::{FieldError, PortFuture, TreeError, TreeStore};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::error::ApiError;
use crate::hub::TopicSource;
use crate::state::ApiState;

pub(crate) use accounts::routes as account_routes;
pub(crate) use folders::routes as folder_routes;
pub(crate) use personas::routes as persona_routes;
pub(crate) use sessions::routes as session_routes;

pub(crate) fn store(state: &ApiState) -> Result<Arc<dyn TreeStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.tree.clone())
        .ok_or_else(|| ApiError::core_down("Session store unavailable"))
}

pub(crate) fn map_err(e: TreeError) -> ApiError {
    match e {
        TreeError::NotFound => ApiError::new(404, "not_found", "Object not found"),
        TreeError::Conflict(reason) => ApiError::conflict(reason),
        TreeError::Invalid(fields) => ApiError::validation_failed(&fields),
        TreeError::Unavailable => ApiError::core_down("Session store unavailable"),
    }
}

pub(crate) fn invalid(fields: Vec<FieldError>) -> ApiError {
    ApiError::validation_failed(&fields)
}

/// `{id}` is an integer (docs/reference/api.md 5); anything else is `400`.
pub(crate) fn parse_id(raw: &str) -> Result<i64, ApiError> {
    raw.parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| ApiError::bad_request("id must be a positive integer"))
}

pub(crate) fn created(v: Value) -> Response {
    (StatusCode::CREATED, Json(v)).into_response()
}

pub(crate) fn ok(v: Value) -> Response {
    Json(v).into_response()
}

pub(crate) fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// `snapshot` of the `sessions` and `connections` topics.
pub(crate) struct TreeTopic(pub Arc<dyn TreeStore>);

impl TopicSource for TreeTopic {
    fn snapshot<'a>(&'a self, topic: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            match topic {
                "sessions" => self.0.sessions_snapshot().await.ok(),
                "connections" => self.0.connections_snapshot().await.ok(),
                _ => None,
            }
        })
    }
}

/// Subscriptions to `sessions` and `connections` (scope `read`); `snapshot` comes from the store.
pub(crate) fn register_topics(hub: &crate::hub::Hub, tree: Arc<dyn TreeStore>) {
    for topic in ["sessions", "connections"] {
        hub.register_topic(
            topic,
            agentd_core::Scope::Read,
            Arc::new(TreeTopic(tree.clone())),
        );
    }
}
