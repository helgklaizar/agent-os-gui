//! Envelope layer: `X-Request-Id`, `X-Agentd-Version`, `Origin`/`Host` check over TCP, building error bodies.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::FutureExt;

use crate::access::{MatchedRoute, UserScope};
use crate::error::ApiError;
use crate::peer::{PeerInfo, Transport};
use crate::state::ApiState;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WS_PATH: &str = "/v1/ws";

/// Request identifier available to handlers (`Extension<RequestId>`).
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// Exactly the client generator format: UUID v7 in canonical form. Anything else is replaced with our own.
fn valid_request_id(v: &str) -> bool {
    v.len() == 36 && uuid::Uuid::parse_str(v).is_ok_and(|u| u.get_version_num() == 7)
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| valid_request_id(v))
        .map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_string)
}

/// `Host` and `Origin` against the allowlist (ADR-0003). Non-browser clients have no `Origin` and are allowed; a present one must
/// be in the list explicitly, so pages from WebKit (`file://`, `null`, `localhost`) are rejected without permission.
pub(crate) fn origin_host_allowed(state: &ApiState, peer: &PeerInfo, headers: &HeaderMap) -> bool {
    if peer.transport != Transport::Tcp {
        return true;
    }
    let Some(port) = peer.local_port else {
        return false;
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let host_ok = host.is_some_and(|h| {
        h.eq_ignore_ascii_case(&format!("127.0.0.1:{port}"))
            || h.eq_ignore_ascii_case(&format!("localhost:{port}"))
            || state
                .policy
                .allowed_hosts
                .iter()
                .any(|a| a.eq_ignore_ascii_case(h))
    });
    let origin_ok = headers.get(header::ORIGIN).is_none_or(|o| {
        o.to_str().ok().is_some_and(|o| {
            state.policy.allowed_origins.iter().any(|a| a == o)
                || state.policy.dev_cors.iter().any(|a| a == o)
                // Own origin (the page is served by this same agent): scheme and `Host` match the already verified host.
                || (host_ok
                    && host.is_some_and(|h| o == format!("http://{h}") || o == format!("https://{h}")))
        })
    });
    host_ok && origin_ok
}

fn set_common_headers(resp: &mut Response, rid: &str) {
    if let Ok(v) = HeaderValue::from_str(rid) {
        resp.headers_mut().insert("x-request-id", v);
    }
    resp.headers_mut()
        .insert("x-agentd-version", HeaderValue::from_static(VERSION));
}

/// Route label for metrics: the template from the table, otherwise a class (`/health`, `/ws`, `unmatched`).
fn route_label(path: &str, matched: Option<&MatchedRoute>) -> String {
    if let Some(m) = matched {
        return m.0.template.to_string();
    }
    match path {
        "/v1/health" => "/health".to_string(),
        WS_PATH => "/ws".to_string(),
        _ => "unmatched".to_string(),
    }
}

pub(crate) async fn envelope(
    State(state): State<Arc<ApiState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let started = std::time::Instant::now();
    let rid = request_id(req.headers());
    req.extensions_mut().insert(RequestId(rid.clone()));
    let peer = req
        .extensions()
        .get::<ConnectInfo<PeerInfo>>()
        .map(|c| c.0.clone());
    let is_ws = req.uri().path() == WS_PATH;
    let path = req.uri().path().to_string();
    // For WS the handler does the check: the response is an upgrade and close `4403`, not HTTP 403.
    let blocked = !is_ws && peer.is_some_and(|p| !origin_host_allowed(&state, &p, req.headers()));
    let mut resp = if blocked {
        tracing::warn!(scope = "api", event = "forbidden_origin", request_id = %rid, "Origin or Host not in the list");
        ApiError::forbidden_origin().into_response()
    } else {
        // A handler panic does not drop the connection: the client gets `500 internal` with `request_id`.
        match AssertUnwindSafe(next.run(req)).catch_unwind().await {
            Ok(r) => r,
            Err(_) => {
                tracing::error!(scope = "api", event = "handler_panic", request_id = %rid, "handler panic");
                ApiError::internal().into_response()
            }
        }
    };
    let error = resp.extensions().get::<ApiError>().cloned();
    if let Some(err) = &error {
        let bytes = serde_json::to_vec(&err.body(&rid)).unwrap_or_default();
        *resp.body_mut() = Body::from(bytes);
        resp.headers_mut().remove(header::CONTENT_LENGTH);
        if let Some(v) = err.retry_after_header() {
            resp.headers_mut().insert(header::RETRY_AFTER, v);
        }
        if let Some(v) = err.allow_header() {
            resp.headers_mut().insert(header::ALLOW, v);
        }
    }
    set_common_headers(&mut resp, &rid);
    let status = resp.status().as_u16();
    let dur_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let label = route_label(&path, resp.extensions().get::<MatchedRoute>());
    state
        .metrics
        .record(&label, status, error.as_ref().map(|e| e.code), dur_ms);
    let user_scope = resp.extensions().get::<UserScope>().map_or("none", |u| u.0);
    if status >= 400 {
        tracing::info!(scope = "api", event = "http", request_id = %rid, user_scope, status, dur_ms, "request rejected");
    } else {
        tracing::debug!(scope = "api", event = "http", request_id = %rid, user_scope, status, dur_ms, "request");
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::valid_request_id;

    #[test]
    fn request_id_accepts_only_canonical_uuid_v7() {
        assert!(valid_request_id(&uuid::Uuid::now_v7().to_string()));
        assert!(!valid_request_id("f47ac10b-58cc-4372-a567-0e02b2c3d479"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("garbage"));
        assert!(!valid_request_id(
            &uuid::Uuid::now_v7().simple().to_string()
        ));
    }
}
