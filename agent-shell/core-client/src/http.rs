//! Low-level HTTP: request and raw response.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::api::ApiError;
use crate::error::ClientError;

/// A request to the core. The path is complete, with the `/v1` prefix and query string.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
    /// Raw body with a content type (file upload); if set, `body` is empty.
    pub raw: Option<(String, Vec<u8>)>,
    pub headers: Vec<(String, String)>,
    /// `false` means no `Authorization` header (anonymous `GET /v1/health`).
    pub authorized: bool,
}

impl Request {
    fn new(method: &str, path: &str, body: Option<Value>) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            body,
            raw: None,
            headers: Vec::new(),
            authorized: true,
        }
    }

    pub fn get(path: &str) -> Request {
        Request::new("GET", path, None)
    }

    pub fn post(path: &str, body: Value) -> Request {
        Request::new("POST", path, Some(body))
    }

    pub fn put(path: &str, body: Value) -> Request {
        Request::new("PUT", path, Some(body))
    }

    pub fn patch(path: &str, body: Value) -> Request {
        Request::new("PATCH", path, Some(body))
    }

    pub fn delete(path: &str) -> Request {
        Request::new("DELETE", path, None)
    }

    /// `POST multipart/form-data` with a single `file` field (api.md section 5.2, attachments).
    pub fn multipart(path: &str, filename: &str, mime: &str, data: &[u8]) -> Request {
        let clean = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if matches!(c, '"' | '\\' | '\r' | '\n') {
                        '_'
                    } else {
                        c
                    }
                })
                .collect()
        };
        let boundary = format!("----agentos-{}", uuid::Uuid::now_v7().simple());
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            clean(filename),
            clean(mime)
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let mut r = Request::new("POST", path, None);
        r.raw = Some((format!("multipart/form-data; boundary={boundary}"), body));
        r
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn anonymous(mut self) -> Request {
        self.authorized = false;
        self
    }
}

/// A response without status parsing.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    /// Header by name, case-insensitive.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, ClientError> {
        serde_json::from_slice(&self.body)
            .map_err(|_| ClientError::Protocol("response body not parsed".into()))
    }

    /// Error in the api.md section 3 format (for statuses >= 400).
    pub fn api_error(&self) -> ApiError {
        ApiError::from_response(
            self.status,
            &self.body,
            self.header("x-request-id"),
            self.header("retry-after"),
        )
    }
}
