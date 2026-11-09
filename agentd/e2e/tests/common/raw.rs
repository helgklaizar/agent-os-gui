//! Raw HTTP over the core unix socket: uploading and downloading binary bodies (the core client only sends JSON).
#![allow(dead_code)]

use std::path::Path;

use serde_json::{Value, json};

use super::Env;

pub struct RawReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawReply {
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

pub async fn raw(
    sock: &Path,
    token: &str,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> RawReply {
    raw_hdr(sock, token, method, path, content_type, &[], body).await
}

pub async fn raw_hdr(
    sock: &Path,
    token: &str,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    extra: &[(&str, &str)],
    body: &[u8],
) -> RawReply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::UnixStream::connect(sock).await.unwrap();
    let mut head = format!(
        "{method} /v1{path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(ct) = content_type {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    // The server may answer `413` and close the connection before reading the whole body: a write error does not fail the test.
    drop(s.write_all(body).await);
    let mut out = Vec::new();
    drop(s.read_to_end(&mut out).await);
    let split = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response without headers")
        + 4;
    let text = String::from_utf8_lossy(&out[..split]).into_owned();
    let mut lines = text.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect::<Vec<_>>();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let mut body = out[split..].to_vec();
    if chunked {
        body = dechunk(&body);
    }
    RawReply {
        status,
        headers,
        body,
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
        let n =
            usize::from_str_radix(std::str::from_utf8(&b[..i]).unwrap().trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[i + 2 + n + 2..];
    }
    out
}

/// `multipart/form-data` body with a single `file` field.
pub fn multipart(filename: &str, mime: &str, data: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----e2eBoundary7f3a";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Env {
    pub async fn upload(
        &self,
        session_id: i64,
        filename: &str,
        mime: &str,
        data: &[u8],
    ) -> RawReply {
        let (ct, body) = multipart(filename, mime, data);
        raw(
            &self.sock,
            &self.chat_token,
            "POST",
            &format!("/sessions/{session_id}/attachments"),
            Some(&ct),
            &body,
        )
        .await
    }

    pub async fn download(&self, id: i64) -> RawReply {
        raw(
            &self.sock,
            &self.read_token,
            "GET",
            &format!("/attachments/{id}"),
            None,
            &[],
        )
        .await
    }

    /// Creates a session as `chat`; returns its id.
    pub async fn session(&self, name: &str) -> i64 {
        self.chat.make("/sessions", json!({"name": name})).await["id"]
            .as_i64()
            .unwrap()
    }
}
