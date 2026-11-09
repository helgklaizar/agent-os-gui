//! Chat: messages, history, attachments, clearing (docs/reference/api.md 5.2) and the `session.<id>` topic.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{
    ChatError, ChatStore, MAX_ATTACHMENT_BYTES, MAX_BODY_BYTES, MAX_PAGE, Posted,
    parse_user_message,
};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get, post};
use serde_json::Value;

use crate::body::read_json_limited;
use crate::error::ApiError;
use crate::hub::TopicSource;
use crate::state::ApiState;
use crate::tree::{created, invalid, no_content, ok, parse_id};

type St = State<Arc<ApiState>>;

/// JSON message body: 1 MiB of text plus framing.
const MESSAGE_BODY_LIMIT: usize = MAX_BODY_BYTES + 64 * 1024;
/// Attachment upload body: the file plus multipart framing.
const UPLOAD_BODY_LIMIT: usize = MAX_ATTACHMENT_BYTES + 64 * 1024;

pub(crate) fn store(state: &ApiState) -> Result<Arc<dyn ChatStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.chat.clone())
        .ok_or_else(|| ApiError::core_down("Chat store unavailable"))
}

pub(crate) fn map_err(e: ChatError) -> ApiError {
    match e {
        ChatError::NotFound => ApiError::new(404, "not_found", "Object not found"),
        ChatError::Invalid(f) => invalid(f),
        ChatError::Conflict(code) => {
            ApiError::new(409, code, "client_id is already used by another text")
        }
        ChatError::TooLarge => ApiError::payload_too_large(MAX_ATTACHMENT_BYTES),
        ChatError::QueueFull => {
            ApiError::new(429, "rate_limited", "The session message queue is full").retry_after(2)
        }
        ChatError::Unavailable => ApiError::core_down("Chat store unavailable"),
    }
}

fn query(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

async fn history(
    State(state): St,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let q = query(raw);
    let before = q.get("before").map(|v| parse_id(v)).transpose()?;
    let limit = match q.get("limit") {
        None => 50,
        Some(v) => v
            .parse::<i64>()
            .ok()
            .filter(|n| (1..=MAX_PAGE).contains(n))
            .ok_or_else(|| ApiError::bad_request("limit: 1 to 200"))?,
    };
    Ok(ok(store.list(id, before, limit).await.map_err(map_err)?))
}

async fn send(State(state): St, Path(id): Path<String>, body: Body) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let msg =
        parse_user_message(&read_json_limited(body, MESSAGE_BODY_LIMIT).await?).map_err(map_err)?;
    // TODO(A4-03): `202` if the `send` action is in `ask` mode.
    let posted = match responder(&state) {
        Some(svc) => svc.send(id, msg).await,
        None => store.post_user(id, msg).await,
    };
    Ok(match posted.map_err(map_err)? {
        Posted::Created(m) => created(m),
        Posted::Existing(m) => ok(m),
    })
}

fn responder(state: &ApiState) -> Option<agentd_core::ChatService> {
    state.services().and_then(|s| s.responder.clone())
}

/// `POST /sessions/{id}/stop`: interrupts the current reply; with no active reply it is also `204` (idempotent).
async fn stop(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    store.context(id).await.map_err(map_err)?;
    if let Some(svc) = responder(&state) {
        let stopped = svc.stop(id);
        tracing::info!(
            scope = "chat",
            event = "stop",
            session_id = id,
            stopped,
            "reply stop"
        );
    }
    Ok(no_content())
}

async fn clear(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    // TODO(A4-03): `202` if the `delete` action is in `ask` mode.
    store.clear(id).await.map_err(map_err)?;
    Ok(no_content())
}

async fn upload(
    State(state): St,
    Path(id): Path<String>,
    mut mp: Multipart,
) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let mp_err = |e: axum::extract::multipart::MultipartError| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::payload_too_large(MAX_ATTACHMENT_BYTES)
        } else {
            ApiError::bad_request("multipart could not be parsed")
        }
    };
    while let Some(field) = mp.next_field().await.map_err(mp_err)? {
        if field.name() != Some("file") {
            continue;
        }
        let name = field.file_name().unwrap_or("file").to_string();
        let mime = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let data = field.bytes().await.map_err(mp_err)?;
        let att = store
            .put_attachment(id, name, mime, data.to_vec())
            .await
            .map_err(map_err)?;
        return Ok(created(att.to_json()));
    }
    Err(invalid(vec![agentd_core::FieldError::new(
        "file",
        "field file is required",
    )]))
}

/// Name for the header: only safe ASCII characters.
fn ascii_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    if n.trim_matches('_').is_empty() {
        "file".into()
    } else {
        n
    }
}

async fn download(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (store, id) = (store(&state)?, parse_id(&id)?);
    let (att, data) = store.get_attachment(id).await.map_err(map_err)?;
    let mut resp = Response::new(Body::from(data));
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        ascii_name(&att.name)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp.into_response())
}

pub(crate) struct Routes {
    pub messages: MethodRouter<Arc<ApiState>>,
    pub clear: MethodRouter<Arc<ApiState>>,
    pub stop: MethodRouter<Arc<ApiState>>,
    pub upload: MethodRouter<Arc<ApiState>>,
    pub download: MethodRouter<Arc<ApiState>>,
}

pub(crate) fn routes() -> Routes {
    Routes {
        messages: get(history).post(send),
        clear: post(clear),
        stop: post(stop),
        upload: post(upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        download: get(download),
    }
}

/// `snapshot` of the `session.<id>` topic: the session, the last page of messages, `open_requests` (A4), `tasks` (A5).
pub(crate) struct SessionTopic(pub Arc<dyn ChatStore>);

impl TopicSource for SessionTopic {
    fn snapshot<'a>(&'a self, topic: &'a str) -> agentd_core::PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            let id: i64 = topic
                .strip_prefix("session.")?
                .parse()
                .ok()
                .filter(|n| *n > 0)?;
            self.0.session_snapshot(id).await.ok()
        })
    }
}
