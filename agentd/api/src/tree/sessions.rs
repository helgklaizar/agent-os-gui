//! `/sessions`: sessions, threads, `read`.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::tree::{parse_session, parse_thread};
use agentd_core::{Principal, SessionFilter};
use axum::body::Body;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get, post};
use serde_json::Value;

use super::{created, invalid, map_err, no_content, ok, parse_id, store};
use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;

type St = State<Arc<ApiState>>;

fn opt_id(q: &HashMap<String, String>, key: &str) -> Result<Option<i64>, ApiError> {
    q.get(key).map(|v| parse_id(v)).transpose()
}

fn filter(q: &HashMap<String, String>) -> Result<SessionFilter, ApiError> {
    let archived = match q.get("archived").map(String::as_str) {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => return Err(ApiError::bad_request("archived: true or false")),
    };
    Ok(SessionFilter {
        folder_id: opt_id(q, "folder_id")?,
        parent_id: opt_id(q, "parent_id")?,
        archived,
    })
}

/// Query string `a=1&b=2` without percent-encoding: filters accept only numbers and `true`/`false`.
fn query_pairs(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

async fn list(State(state): St, RawQuery(raw): RawQuery) -> Result<Response, ApiError> {
    let (store, f) = (store(&state)?, filter(&query_pairs(raw))?);
    Ok(ok(store.list_sessions(f).await.map_err(map_err)?))
}

async fn create(State(state): St, body: Body) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let input = parse_session(&read_json(body).await?, true).map_err(invalid)?;
    Ok(created(store.create_session(input).await.map_err(map_err)?))
}

async fn get_one(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    Ok(ok(store.get_session(id).await.map_err(map_err)?))
}

async fn patch(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let input = parse_session(&read_json(body).await?, false).map_err(invalid)?;
    // Session permissions: every `cfg.perms` pair passes the "tighten only" check before settings are written.
    if let Some(Value::Object(perms)) = input.cfg.as_ref().and_then(|c| c.get("perms")) {
        crate::permissions::apply_session_perms(&state, &who, id, perms).await?;
    }
    Ok(ok(store.patch_session(id, input).await.map_err(map_err)?))
}

async fn delete(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    // Open requests for the session's permissions are closed with a denial before the row is deleted (docs/reference/api.md 5.9).
    if let Some(gate) = state.services().and_then(|s| s.gate.clone()) {
        gate.requests().close_session(id).await;
    }
    if let Some(browser) = state.services().and_then(|s| s.browser.clone()) {
        browser.close_session(id);
    }
    store.delete_session(id).await.map_err(map_err)?;
    Ok(no_content())
}

async fn thread(
    State(state): St,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let name = parse_thread(&read_json(body).await?).map_err(invalid)?;
    Ok(created(
        store.create_thread(id, name).await.map_err(map_err)?,
    ))
}

/// The body of `POST /sessions/{id}/read` is unused: anything is accepted, including empty.
async fn read(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    store.mark_read(id).await.map_err(map_err)?;
    Ok(no_content())
}

pub(crate) fn routes() -> [MethodRouter<Arc<ApiState>>; 4] {
    [
        get(list).post(create),
        get(get_one).patch(patch).delete(delete),
        post(thread),
        post(read),
    ]
}
