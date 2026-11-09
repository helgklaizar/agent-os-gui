//! Browser client (ADR-0010): serving statics from `--web-root`, security headers, dev-CORS and the WS token
//! from the subprotocol. Statics are anonymous (they are code, not data); data still requires a token.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::ApiState;

/// WS subprotocol: the client offers `agentd.v1` and `bearer.<token>`, the core picks only `agentd.v1`.
pub(crate) const WS_PROTOCOL: &str = "agentd.v1";
const BEARER_PREFIX: &str = "bearer.";
const MAX_FILE: u64 = 32 * 1024 * 1024;

const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; \
connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// Token from `Sec-WebSocket-Protocol: agentd.v1, bearer.<token>` (a browser cannot set `Authorization`).
pub(crate) fn ws_protocol_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all("sec-websocket-protocol")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .find_map(|p| p.strip_prefix(BEARER_PREFIX))
        .filter(|t| !t.is_empty())
}

/// Static path: `/` and `/app/...` (GET and HEAD).
pub(crate) fn is_static(method: &Method, path: &str) -> bool {
    matches!(*method, Method::GET | Method::HEAD) && (path == "/" || path.starts_with("/app/"))
}

/// Relative path inside the root; anything other than ordinary names (`..`, `/`, `\`, NUL) is rejected.
fn relative(path: &str) -> Option<PathBuf> {
    let rest = if path == "/" {
        "index.html"
    } else {
        path.strip_prefix("/app/")?
    };
    let rest = if rest.is_empty() || rest.ends_with('/') {
        format!("{rest}index.html")
    } else {
        rest.to_string()
    };
    if rest.contains(['\\', '\0']) {
        return None;
    }
    let p = PathBuf::from(&rest);
    p.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then_some(p)
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "ftl" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn not_found() -> Response {
    let mut r = Response::new(Body::from("not found"));
    *r.status_mut() = StatusCode::NOT_FOUND;
    security_headers(&mut r);
    r
}

fn security_headers(r: &mut Response) {
    let h = r.headers_mut();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}

/// Serves a file from the root; escaping the root (including via a symlink) gives `404`.
pub(crate) async fn serve(
    state: &ApiState,
    method: &Method,
    path: &str,
    if_none_match: Option<&str>,
) -> Response {
    let Some(root) = state.policy.web_root.as_ref() else {
        return not_found();
    };
    let Some(rel) = relative(path) else {
        return not_found();
    };
    let (Ok(root), Ok(file)) = (
        tokio::fs::canonicalize(root).await,
        tokio::fs::canonicalize(root.join(&rel)).await,
    ) else {
        return not_found();
    };
    if !file.starts_with(&root) {
        return not_found();
    }
    let Ok(meta) = tokio::fs::metadata(&file).await else {
        return not_found();
    };
    if !meta.is_file() || meta.len() > MAX_FILE {
        return not_found();
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let etag = format!("\"{:x}-{:x}\"", meta.len(), mtime);
    let mut resp = if if_none_match == Some(etag.as_str()) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let Ok(bytes) = tokio::fs::read(&file).await else {
            return not_found();
        };
        let mut r = if method == Method::HEAD {
            Response::new(Body::empty())
        } else {
            Response::new(Body::from(bytes))
        };
        if let Ok(v) = HeaderValue::from_str(content_type(&file)) {
            r.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        r
    };
    if let Ok(v) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    security_headers(&mut resp);
    resp
}

/// Dev-CORS: only the listed origins (`--dev-cors`); without a list there are no CORS headers, `*` is impossible.
pub(crate) async fn cors(State(state): State<Arc<ApiState>>, req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| state.policy.dev_cors.iter().any(|a| a == o))
        .map(str::to_string);
    let Some(origin) = origin else {
        return next.run(req).await;
    };
    let preflight = req.method() == Method::OPTIONS
        && req.headers().contains_key("access-control-request-method");
    let mut resp = if preflight {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NO_CONTENT;
        let h = r.headers_mut();
        h.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
        );
        h.insert(
            "access-control-allow-headers",
            HeaderValue::from_static("Authorization, Content-Type, X-Request-Id, If-Match"),
        );
        h.insert("access-control-max-age", HeaderValue::from_static("600"));
        r
    } else {
        next.run(req).await
    };
    if let Ok(v) = HeaderValue::from_str(&origin) {
        resp.headers_mut().insert("access-control-allow-origin", v);
    }
    resp.headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Origin"));
    resp.headers_mut().insert(
        "access-control-expose-headers",
        HeaderValue::from_static("X-Request-Id, ETag, Retry-After"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        assert_eq!(relative("/"), Some(PathBuf::from("index.html")));
        assert_eq!(relative("/app/"), Some(PathBuf::from("index.html")));
        assert_eq!(relative("/app/a/b.js"), Some(PathBuf::from("a/b.js")));
        for bad in [
            "/app/../x",
            "/app/a/../../x",
            "/app//etc/passwd",
            "/app/a\\b",
            "/app/%2e%2e/x",
        ] {
            let r = relative(bad);
            assert!(
                r.is_none() || !r.unwrap().components().any(|c| c == Component::ParentDir),
                "{bad}"
            );
        }
        assert_eq!(relative("/app//etc/passwd"), None);
        assert_eq!(relative("/app/../x"), None);
    }

    #[test]
    fn protocol_token() {
        let mut h = HeaderMap::new();
        h.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("agentd.v1, bearer.agt_abc-_9"),
        );
        assert_eq!(ws_protocol_token(&h), Some("agt_abc-_9"));
        let mut h = HeaderMap::new();
        h.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("agentd.v1"),
        );
        assert_eq!(ws_protocol_token(&h), None);
        h.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("bearer."),
        );
        assert_eq!(ws_protocol_token(&h), None);
    }

    #[test]
    fn content_types_by_extension() {
        let ct = |n: &str| content_type(Path::new(n));
        assert_eq!(ct("a.html"), "text/html; charset=utf-8");
        assert_eq!(ct("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(ct("a.mjs"), "text/javascript; charset=utf-8");
        assert_eq!(ct("a.wasm"), "application/wasm");
        assert_eq!(ct("a.css"), "text/css; charset=utf-8");
        assert_eq!(ct("a.svg"), "image/svg+xml");
        assert_eq!(ct("a.png"), "image/png");
        assert_eq!(ct("a.ico"), "image/x-icon");
        assert_eq!(ct("a.json"), "application/json");
        assert_eq!(ct("a.ftl"), "text/plain; charset=utf-8");
        assert_eq!(ct("a.bin"), "application/octet-stream");
        assert_eq!(ct("noext"), "application/octet-stream");
    }

    #[test]
    fn size_limit_is_32_mib() {
        assert_eq!(MAX_FILE, 33_554_432);
    }

    #[test]
    fn static_paths() {
        assert!(is_static(&Method::GET, "/"));
        assert!(is_static(&Method::HEAD, "/app/x.js"));
        assert!(!is_static(&Method::POST, "/"));
        assert!(!is_static(&Method::GET, "/v1/health"));
        assert!(!is_static(&Method::GET, "/application"));
    }
}
