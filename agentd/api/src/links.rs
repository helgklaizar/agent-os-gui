//! `GET`/`PUT /v1/sessions/{id}/bookmarks` and `/v1/browser/quicklinks` (docs/reference/api.md 5.8): lists of `[{title, url}]`,
//! `ETag` and `If-Match` (`412` on `rev` mismatch, `428` without the header).

use std::sync::Arc;

use agentd_core::{
    QUICKLINKS_KEY, SettingsDoc, SettingsError, SettingsStore, bookmarks_key, default_quicklinks,
    validate_links,
};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use serde_json::json;

use crate::body::{if_match_rev, read_json};
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, map_err as tree_err, parse_id, store as tree_store};

type St = State<Arc<ApiState>>;

fn settings(state: &ApiState) -> Result<Arc<dyn SettingsStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.settings.clone())
        .ok_or_else(|| ApiError::core_down("Settings store unavailable"))
}

fn map_err(e: SettingsError) -> ApiError {
    match e {
        SettingsError::Stale { current } => ApiError::precondition_failed(current),
        SettingsError::Invalid(f) => ApiError::validation_failed(&f),
        SettingsError::UnknownPage => ApiError::not_found(),
        SettingsError::ReadOnly => ApiError::permission_denied("read_only"),
        SettingsError::Unavailable => ApiError::core_down("Settings store unavailable"),
    }
}

fn reply(d: SettingsDoc) -> Response {
    let mut resp = Json(d.doc).into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("\"{}\"", d.rev)) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp
}

async fn session_exists(state: &ApiState, id: i64) -> Result<(), ApiError> {
    tree_store(state)?
        .get_session(id)
        .await
        .map(|_| ())
        .map_err(tree_err)
}

async fn put(
    state: &ApiState,
    key: &str,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let store = settings(state)?;
    let rev = if_match_rev(headers)?;
    let list = read_json(body).await?;
    let errors = validate_links(&list);
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    Ok(reply(
        store.put_list(key, list, rev).await.map_err(map_err)?,
    ))
}

async fn get_bookmarks(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let id = parse_id(&id)?;
    session_exists(&state, id).await?;
    let d = settings(&state)?
        .get_list(&bookmarks_key(id), json!([]))
        .await
        .map_err(map_err)?;
    Ok(reply(d))
}

async fn put_bookmarks(
    State(state): St,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let id = parse_id(&id)?;
    session_exists(&state, id).await?;
    put(&state, &bookmarks_key(id), &headers, body).await
}

async fn get_quicklinks(State(state): St) -> Result<Response, ApiError> {
    let d = settings(&state)?
        .get_list(QUICKLINKS_KEY, default_quicklinks())
        .await
        .map_err(map_err)?;
    Ok(reply(d))
}

async fn put_quicklinks(
    State(state): St,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    put(&state, QUICKLINKS_KEY, &headers, body).await
}

pub(crate) fn bookmarks_route() -> MethodRouter<Arc<ApiState>> {
    get(get_bookmarks).put(put_bookmarks)
}

pub(crate) fn quicklinks_route() -> MethodRouter<Arc<ApiState>> {
    get(get_quicklinks).put(put_quicklinks)
}
