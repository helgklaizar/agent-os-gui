//! `/events` (docs/reference/api.md 5.3): feed, counters, mark as read, subtypes; the `events` topic.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{EventError, EventStore, PortFuture, parse_filter, parse_read, subtypes_json};
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get, post};
use serde_json::{Value, json};

use crate::body::read_bytes_limited;
use crate::error::ApiError;
use crate::hub::TopicSource;
use crate::state::ApiState;
use crate::tree::{invalid, no_content, ok, parse_id};

type St = State<Arc<ApiState>>;

fn store(state: &ApiState) -> Result<Arc<dyn EventStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.events.clone())
        .ok_or_else(|| ApiError::core_down("Event feed unavailable"))
}

fn map_err(e: EventError) -> ApiError {
    match e {
        EventError::NotFound => ApiError::new(404, "not_found", "Event not found"),
        EventError::Invalid(f) => invalid(f),
        EventError::Unavailable => ApiError::core_down("Event feed unavailable"),
    }
}

/// Query string `a=1&b=2`: filters are numbers, `true`/`false` and subtype names, no percent-encoding.
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

async fn list(State(state): St, RawQuery(raw): RawQuery) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let filter = parse_filter(&query_pairs(raw)).map_err(invalid)?;
    Ok(ok(store.list(filter).await.map_err(map_err)?))
}

/// The body of `POST /events/read` is optional: empty means all unread.
async fn read_many(State(state): St, body: Body) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let bytes = read_bytes_limited(body, crate::body::BODY_LIMIT).await?;
    let json: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::bad_request("Request body is not JSON"))?
    };
    let selector = parse_read(&json).map_err(invalid)?;
    let marked = store.mark_read(selector).await.map_err(map_err)?;
    Ok(ok(json!({ "marked": marked })))
}

async fn read_one(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    store.mark_one(id).await.map_err(map_err)?;
    Ok(no_content())
}

async fn subtypes() -> Response {
    ok(subtypes_json())
}

pub(crate) fn routes() -> [MethodRouter<Arc<ApiState>>; 4] {
    [get(list), post(read_many), get(subtypes), post(read_one)]
}

/// `snapshot` of the `events` topic: the latest events and counters.
pub(crate) struct EventsTopic(pub Arc<dyn EventStore>);

impl TopicSource for EventsTopic {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move { self.0.snapshot().await.ok() })
    }
}

/// `snapshot` of the `bar.indicators` topic: `{items: [IndicatorState]}`.
pub(crate) struct IndicatorsTopic(pub agentd_core::IndicatorEngine);

impl TopicSource for IndicatorsTopic {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move { self.0.store().snapshot().await.ok() })
    }
}
