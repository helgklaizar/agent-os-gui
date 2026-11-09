//! `/files` (docs/reference/api.md 5.7): editing `AGENT.md` and `memory/`. Only `admin` and the local socket (route table).
//! The path is checked by `agentd_core::normalize` before touching the disk; the response is `403 permission_denied` for anything else.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{
    FieldError, FileError, FileInfo, FilePath, FileStore, MAX_FILE_BYTES, Principal, normalize,
};
use axum::body::Body;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use serde_json::Value;

use crate::body::read_bytes_limited;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::ok;

fn store(state: &ApiState) -> Result<Arc<dyn FileStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.files.clone())
        .ok_or_else(|| ApiError::core_down("Files unavailable"))
}

fn map_err(e: FileError) -> ApiError {
    match e {
        FileError::Denied => ApiError::permission_denied("path_not_allowed"),
        FileError::NotFound => ApiError::new(404, "not_found", "File not found"),
        FileError::Stale(cur) => ApiError::precondition_failed_hash(&cur),
        FileError::TooLarge => ApiError::payload_too_large(MAX_FILE_BYTES),
        FileError::Unavailable => ApiError::core_down("Files unavailable"),
    }
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `path` parameter: missing or not an allowed one gives `403` (the path is not revealed in the response).
fn path_of(raw: Option<String>) -> Result<FilePath, ApiError> {
    let q: HashMap<String, String> = raw
        .unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect();
    let Some(path) = q.get("path") else {
        return Err(ApiError::validation_failed(&[FieldError::new(
            "path",
            "parameter path is required",
        )]));
    };
    normalize(path).ok_or_else(|| ApiError::permission_denied("path_not_allowed"))
}

/// `If-Match`: quoted hash, weak form allowed; no header gives `428`.
fn if_match(headers: &HeaderMap) -> Result<String, ApiError> {
    let raw = headers
        .get(header::IF_MATCH)
        .ok_or_else(ApiError::precondition_required)?;
    raw.to_str()
        .ok()
        .map(str::trim)
        .map(|s| {
            s.strip_prefix("W/")
                .unwrap_or(s)
                .trim_matches('"')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request("If-Match: file rev expected"))
}

fn with_etag(mut resp: Response, rev: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(&format!("\"{rev}\"")) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp
}

async fn list(State(state): State<Arc<ApiState>>) -> Result<Response, ApiError> {
    let items = store(&state)?.list().await.map_err(map_err)?;
    Ok(ok(Value::Array(
        items.iter().map(FileInfo::to_json).collect(),
    )))
}

async fn read(
    State(state): State<Arc<ApiState>>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let path = path_of(raw)?;
    let (bytes, info) = store(&state)?.read(path).await.map_err(map_err)?;
    let resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        bytes,
    )
        .into_response();
    Ok(with_etag(resp, &info.rev))
}

async fn write(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let path = path_of(raw)?;
    let expected = if_match(&headers)?;
    let bytes = read_bytes_limited(body, MAX_FILE_BYTES).await?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(ApiError::bad_request("The file must be UTF-8 text"));
    }
    let size = bytes.len();
    let (target, action) = (
        format!("file:{}", path.rel),
        if path.kind == agentd_core::FileKind::Prompt {
            "config_write"
        } else {
            "memory_write"
        },
    );
    let info = store(&state)?
        .write(path, expected, bytes)
        .await
        .map_err(map_err)?;
    // The owner's write goes to the log (the file text is not written, only the size and version hash).
    crate::audit::user_action(
        &state,
        &who,
        action,
        target,
        serde_json::json!({"size": size, "rev": info.rev}),
    )
    .await;
    Ok(with_etag(ok(info.to_json()), &info.rev))
}

pub(crate) fn list_route() -> MethodRouter<Arc<ApiState>> {
    get(list)
}

pub(crate) fn content_route() -> MethodRouter<Arc<ApiState>> {
    get(read).put(write)
}
