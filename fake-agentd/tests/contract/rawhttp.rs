//! Raw HTTP/1.1 over the mock's unix socket: contract tests do not depend on `core-client` (mock and client are checked independently).

use std::path::Path;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

pub struct Req<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub token: Option<&'a str>,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<Value>,
    /// Raw body with a type (multipart).
    pub raw: Option<(String, Vec<u8>)>,
}

impl<'a> Req<'a> {
    pub fn new(method: &'a str, path: &'a str) -> Req<'a> {
        Req {
            method,
            path,
            token: None,
            headers: Vec::new(),
            body: None,
            raw: None,
        }
    }

    pub fn token(mut self, t: &'a str) -> Req<'a> {
        self.token = Some(t);
        self
    }

    pub fn header(mut self, k: &'a str, v: &str) -> Req<'a> {
        self.headers.push((k, v.to_string()));
        self
    }

    #[allow(dead_code)]
    pub fn raw(mut self, content_type: &str, body: Vec<u8>) -> Req<'a> {
        self.raw = Some((content_type.to_string(), body));
        self
    }

    pub fn json(mut self, v: Value) -> Req<'a> {
        self.body = Some(v);
        self
    }
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(pos) = raw.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&raw[..pos])
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok());
        let Some(size) = size else { break };
        if size == 0 {
            break;
        }
        let start = pos + 2;
        if raw.len() < start + size {
            break;
        }
        out.extend_from_slice(&raw[start..start + size]);
        raw = raw.get(start + size + 2..).unwrap_or(&[]);
    }
    out
}

/// Reads a response to its end by `Content-Length`, the final chunk or the end of headers (204): it does not wait for the
/// connection to close, so it does not depend on whether the server closes it after the response.
async fn read_response(stream: &mut UnixStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if response_complete(&raw) {
            return raw;
        }
        let n = stream
            .read(&mut buf)
            .await
            .unwrap_or_else(|e| panic!("read: {e}"));
        if n == 0 {
            return raw;
        }
        raw.extend_from_slice(&buf[..n]);
    }
}

fn response_complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let payload = &raw[split + 4..];
    if let Some(len) = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        return payload.len() >= len;
    }
    if head.contains("transfer-encoding: chunked") {
        return payload.ends_with(b"0\r\n\r\n");
    }
    true
}

pub async fn send(socket: &Path, req: Req<'_>) -> Resp {
    let mut stream = UnixStream::connect(socket)
        .await
        .unwrap_or_else(|e| panic!("mock socket: {e}"));
    let body = req.body.as_ref().map(Value::to_string);
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
        req.method, req.path
    );
    if let Some(t) = req.token {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    for (k, v) in &req.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = &body {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    if let Some((ct, b)) = &req.raw {
        head.push_str(&format!(
            "Content-Type: {ct}\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    head.push_str("\r\n");
    if let Some(b) = &body {
        head.push_str(b);
    }
    let mut wire = head.into_bytes();
    if let Some((_, b)) = &req.raw {
        wire.extend_from_slice(b);
    }
    stream
        .write_all(&wire)
        .await
        .unwrap_or_else(|e| panic!("write: {e}"));
    let raw = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_response(&mut stream),
    )
    .await
    .unwrap_or_else(|_| panic!("no response within 10 s"));
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no end of headers"));
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no status: {head}"));
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let payload = &raw[split + 4..];
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked {
        dechunk(payload)
    } else {
        payload.to_vec()
    };
    Resp {
        status,
        headers,
        body,
    }
}
