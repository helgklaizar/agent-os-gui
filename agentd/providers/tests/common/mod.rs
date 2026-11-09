//! Local test HTTP server: serves a given reply in parts and records requests.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentd_core::{
    ModelAccount, ModelRequest, ProviderEvent, ProviderKind, Secret, SecretStore, Turn, TurnRole,
};
use agentd_providers::KeysFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Reply scenario: status, headers, body parts with a pause before each; `hang` means stay silent after the parts.
#[derive(Clone)]
pub struct Script {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<(Duration, Vec<u8>)>,
    pub hang: bool,
    /// Pause before the response headers.
    pub head_delay: Duration,
}

impl Script {
    pub fn sse(body: &str) -> Script {
        Script {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks: vec![(Duration::ZERO, body.as_bytes().to_vec())],
            hang: false,
            head_delay: Duration::ZERO,
        }
    }
    pub fn status(code: u16) -> Script {
        Script {
            status: code,
            headers: vec![],
            chunks: vec![(
                Duration::ZERO,
                b"{\"error\":\"secret-detail-from-provider\"}".to_vec(),
            )],
            hang: false,
            head_delay: Duration::ZERO,
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Script {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct Server {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub closed: Arc<Mutex<u32>>,
}

pub async fn serve(script: Script) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(Mutex::new(0u32));
    let (s2, c2) = (seen.clone(), closed.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let (script, seen, closed) = (script.clone(), s2.clone(), c2.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or("").to_string();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                let len: usize = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                seen.lock().unwrap().push(Seen {
                    request_line,
                    headers,
                    body,
                });
                tokio::time::sleep(script.head_delay).await;
                let mut head = format!("HTTP/1.1 {} X\r\nconnection: close\r\n", script.status);
                for (k, v) in &script.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                if sock.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for (pause, bytes) in &script.chunks {
                    tokio::time::sleep(*pause).await;
                    if sock.write_all(bytes).await.is_err() || sock.flush().await.is_err() {
                        return;
                    }
                }
                if script.hang {
                    // Wait until the client closes the connection (request cancelled) and count it.
                    let mut b = [0u8; 64];
                    while sock.read(&mut b).await.unwrap_or(0) > 0 {}
                    *closed.lock().unwrap() += 1;
                }
            });
        }
    });
    Server { url, seen, closed }
}

pub fn keys_with(refs: &[(&str, &str)]) -> (tempfile::TempDir, Arc<KeysFile>) {
    let dir = tempfile::tempdir().unwrap();
    let k = Arc::new(KeysFile::new(dir.path().join("keys.env")));
    let rt = tokio::runtime::Handle::current();
    for (r, v) in refs {
        let k = k.clone();
        let (r, v) = (r.to_string(), v.to_string());
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(k.put(&r, Secret::new(v), 1)).unwrap());
        });
    }
    (dir, k)
}

pub fn account(kind: ProviderKind, base_url: &str, secret_ref: Option<&str>) -> ModelAccount {
    ModelAccount {
        id: 1,
        kind,
        label: "t".into(),
        base_url: Some(base_url.into()),
        secret_ref: secret_ref.map(str::to_string),
    }
}

pub fn request(account: ModelAccount) -> ModelRequest {
    ModelRequest {
        account,
        model: "fixture-model".into(),
        system: Some("system text".into()),
        history: vec![Turn {
            role: TurnRole::User,
            text: "question".into(),
        }],
        max_tokens: 256,
        tools: Vec::new(),
    }
}

/// All events of a request until the end of the stream.
pub async fn collect(p: &dyn agentd_core::ModelProvider, req: ModelRequest) -> Vec<ProviderEvent> {
    let (tx, mut rx) = mpsc::channel(1024);
    p.stream(req, tx).await;
    let mut out = Vec::new();
    while let Some(e) = rx.recv().await {
        out.push(e);
    }
    out
}

pub fn text_of(ev: &[ProviderEvent]) -> String {
    ev.iter()
        .filter_map(|e| match e {
            ProviderEvent::Delta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}
