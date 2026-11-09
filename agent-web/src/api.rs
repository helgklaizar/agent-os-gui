//! HTTP to the core via `fetch`: `Authorization: Bearer`, `X-Request-Id` (UUID v7), JSON.
//! The token is taken from page memory and never goes into the URL.

use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, Response};

use crate::util::uuid7;

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub body: Value,
    /// Reply `ETag` (`rev` of the settings page).
    pub etag: Option<String>,
    /// Pause before retry from `Retry-After`, seconds (`429`, `503`).
    pub retry_after: Option<u64>,
}

/// `Retry-After` in seconds: an integer; the core does not support an HTTP date.
pub fn retry_after_secs(h: &str) -> Option<u64> {
    h.trim().parse::<u64>().ok().map(|s| s.max(1))
}

/// Readable failure reason by reply code (the token and body never get into the text).
pub fn status_text(status: u16, retry_after: Option<u64>) -> String {
    match (status, retry_after) {
        (429, Some(s)) => format!("Too many requests: retry in {s} s"),
        (429, None) => "Too many requests: retry later".to_string(),
        (412, _) => "Data changed elsewhere: page reloaded, repeat the edit".to_string(),
        (403, _) => "Not enough permissions for this action".to_string(),
        (413, _) => "File too large".to_string(),
        (422, _) => "Values failed the core's validation".to_string(),
        (404, _) => "The core does not know this object or route".to_string(),
        (s, _) => format!("Core responded {s}"),
    }
}

impl Resp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn js_err(e: &JsValue) -> String {
    e.as_string()
        .unwrap_or_else(|| "network unavailable".to_string())
}

/// Request body.
pub enum Payload<'a> {
    Empty,
    Json(&'a Value),
    /// `multipart/form-data`: the browser sets `Content-Type` with the boundary.
    Form(&'a web_sys::FormData),
}

/// One request. `Err` is network or parsing; `4xx`/`5xx` replies arrive as `Ok(Resp)`.
pub async fn request(
    method: &str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<Resp, String> {
    let payload = body.map_or(Payload::Empty, Payload::Json);
    send(method, path, token, payload, &[]).await
}

/// Request with a body of any kind and extra headers (`If-Match`).
pub async fn send(
    method: &str,
    path: &str,
    token: &str,
    payload: Payload<'_>,
    extra: &[(&str, &str)],
) -> Result<Resp, String> {
    let headers = Headers::new().map_err(|e| js_err(&e))?;
    headers
        .set("Authorization", &format!("Bearer {token}"))
        .map_err(|e| js_err(&e))?;
    headers
        .set("X-Request-Id", &uuid7())
        .map_err(|e| js_err(&e))?;
    for (k, v) in extra {
        headers.set(k, v).map_err(|e| js_err(&e))?;
    }
    let init = RequestInit::new();
    init.set_method(method);
    match payload {
        Payload::Empty => {}
        Payload::Json(b) => {
            headers
                .set("Content-Type", "application/json")
                .map_err(|e| js_err(&e))?;
            init.set_body(&JsValue::from_str(&b.to_string()));
        }
        Payload::Form(f) => init.set_body(f),
    }
    init.set_headers(&headers);
    let req = Request::new_with_str_and_init(path, &init).map_err(|e| js_err(&e))?;
    let win = web_sys::window().ok_or("no window")?;
    let resp: Response = JsFuture::from(win.fetch_with_request(&req))
        .await
        .map_err(|e| js_err(&e))?
        .dyn_into()
        .map_err(|_| "response is not a Response".to_string())?;
    let status = resp.status();
    let text = match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    let etag = resp.headers().get("ETag").ok().flatten();
    let retry_after = resp
        .headers()
        .get("Retry-After")
        .ok()
        .flatten()
        .and_then(|h| retry_after_secs(&h));
    Ok(Resp {
        status,
        body,
        etag,
        retry_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_is_the_2xx_range() {
        let r = |status| Resp {
            status,
            body: Value::Null,
            etag: None,
            retry_after: None,
        };
        assert!(r(200).ok() && r(204).ok() && r(299).ok());
        assert!(!r(199).ok() && !r(300).ok() && !r(404).ok() && !r(500).ok());
    }

    #[test]
    fn retry_after_is_whole_seconds() {
        assert_eq!(retry_after_secs("30"), Some(30));
        assert_eq!(retry_after_secs(" 7 "), Some(7));
        assert_eq!(retry_after_secs("0"), Some(1));
        assert_eq!(retry_after_secs("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(retry_after_secs(""), None);
    }

    #[test]
    fn rate_limit_text_names_the_wait() {
        let t = status_text(429, Some(42));
        assert!(t.contains("42"), "{t}");
        assert!(status_text(429, None).contains("later"));
        assert!(status_text(412, None).contains("changed"));
        assert!(status_text(403, None).contains("permissions"));
        assert!(status_text(413, None).contains("large"));
        assert!(status_text(500, None).contains("500"));
    }
}
