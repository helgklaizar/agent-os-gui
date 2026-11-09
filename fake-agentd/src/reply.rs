//! Mock responses: body, error in the api.md section 3 format, response assembly.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::Response;
use hyper::body::{Body as HyperBody, Bytes, Frame};
use serde_json::{Value, json};

pub const VERSION: &str = "0.0.0-fake";

/// Whole response body (without `http-body-util`: the `fake-agentd` dependency list is closed).
pub struct Body(Option<Bytes>);

impl Body {
    pub fn empty() -> Body {
        Body(None)
    }

    pub fn from_json(v: &Value) -> Body {
        Body(Some(Bytes::from(v.to_string())))
    }

    pub fn from_bytes(b: Vec<u8>) -> Body {
        Body(Some(Bytes::from(b)))
    }
}

impl HyperBody for Body {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Ready(self.0.take().map(|b| Ok(Frame::data(b))))
    }
}

/// Error per the api.md section 3 table.
#[derive(Debug, Clone)]
pub struct ApiErr {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    pub retry_after: Option<u64>,
}

impl ApiErr {
    pub fn new(status: u16, code: &'static str, message: &str) -> ApiErr {
        ApiErr {
            status,
            code,
            message: message.to_string(),
            details: json!({}),
            retry_after: None,
        }
    }

    pub fn details(mut self, details: Value) -> ApiErr {
        self.details = details;
        self
    }

    pub fn retry_after(mut self, secs: u64) -> ApiErr {
        self.retry_after = Some(secs);
        self
    }

    pub fn body(&self, request_id: &str) -> Value {
        json!({ "error": {
            "code": self.code,
            "message": self.message,
            "details": self.details,
            "retryable": matches!(self.status, 429 | 500 | 503),
            "request_id": request_id,
        } })
    }
}

/// Successful handler response.
pub struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    pub headers: Vec<(&'static str, String)>,
    /// Binary body with a type (attachment download); replaces `body`.
    pub raw: Option<(String, Vec<u8>)>,
}

impl Reply {
    pub fn bytes(status: u16, content_type: &str, data: Vec<u8>) -> Reply {
        Reply {
            status,
            body: None,
            headers: Vec::new(),
            raw: Some((content_type.to_string(), data)),
        }
    }

    pub fn json(status: u16, body: Value) -> Reply {
        Reply {
            status,
            body: Some(body),
            headers: Vec::new(),
            raw: None,
        }
    }

    pub fn no_content() -> Reply {
        Reply {
            status: 204,
            body: None,
            headers: Vec::new(),
            raw: None,
        }
    }

    pub fn header(mut self, name: &'static str, value: String) -> Reply {
        self.headers.push((name, value));
        self
    }
}

/// Any response: the `X-Request-Id` and `X-Agentd-Version` headers are always set (api.md sections 1.2, 2).
pub fn build(
    status: u16,
    request_id: &str,
    body: Option<&Value>,
    headers: &[(&'static str, String)],
) -> Response<Body> {
    let mut b = Response::builder()
        .status(status)
        .header("x-request-id", request_id)
        .header("x-agentd-version", VERSION)
        .header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, v.as_str());
    }
    let body = body.map_or_else(Body::empty, Body::from_json);
    b.body(body)
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

pub fn from_err(e: &ApiErr, request_id: &str) -> Response<Body> {
    let mut headers = Vec::new();
    if let Some(s) = e.retry_after {
        headers.push(("retry-after", s.to_string()));
    }
    build(e.status, request_id, Some(&e.body(request_id)), &headers)
}

pub fn from_reply(r: &Reply, request_id: &str) -> Response<Body> {
    if let Some((ct, data)) = &r.raw {
        let mut b = Response::builder()
            .status(r.status)
            .header("x-request-id", request_id)
            .header("x-agentd-version", VERSION)
            .header("content-type", ct.as_str());
        for (k, v) in &r.headers {
            b = b.header(*k, v.as_str());
        }
        return b
            .body(Body::from_bytes(data.clone()))
            .unwrap_or_else(|_| Response::new(Body::empty()));
    }
    build(r.status, request_id, r.body.as_ref(), &r.headers)
}
