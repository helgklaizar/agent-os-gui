//! MCP over `https`. The server is a test thread: TLS on `rustls` with a one-off certificate and a minimal
//! MCP over Streamable HTTP (one JSON per POST). Trust in a foreign certificate authority is `RmcpConnector::with_ca`
//! (in production, the `AGENTD_MCP_CA_FILE` file); without it a self-signed certificate is rejected.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use agentd_core::{McpConnector, McpError, McpServerSpec, McpTransport};
use agentd_providers::mcp::RmcpConnector;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};

fn spec(url: &str) -> McpServerSpec {
    McpServerSpec {
        name: "remote".into(),
        transport: McpTransport::Http { url: url.into() },
        enabled: true,
        timeout: std::time::Duration::from_secs(10),
        tools: Default::default(),
    }
}

/// MCP server reply to a JSON-RPC request; notifications (no `id`) give `None`.
fn mcp(req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone();
    let p = &req["params"];
    let result = match req["method"].as_str().unwrap_or_default() {
        "initialize" => {
            json!({"protocolVersion": p["protocolVersion"], "capabilities": {"tools": {}},
                               "serverInfo": {"name": "tls-test", "version": "0"}})
        }
        "tools/list" => json!({"tools": [{"name": "echo", "description": "echo",
                                          "inputSchema": {"type": "object"}}]}),
        "tools/call" => json!({"content": [{"type": "text",
                               "text": format!("echo: {}", p["arguments"]["text"].as_str().unwrap_or_default())}],
                               "isError": false}),
        _ => json!({}),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn http_reply(req: &[u8], head_end: usize) -> String {
    let head = String::from_utf8_lossy(&req[..head_end]);
    if !head.starts_with("POST") {
        return "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .into();
    }
    match serde_json::from_slice::<Value>(&req[head_end + 4..])
        .ok()
        .and_then(|v| mcp(&v))
    {
        Some(v) => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{v}",
            v.to_string().len()
        ),
        None => "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    }
}

/// TLS server with a one-off certificate. Returns the port and the certificate PEM.
fn tls_server() -> (u16, String) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let pem = cert.cert.pem();
    let der = CertificateDer::from(cert.cert.der().to_vec());
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![der], key)
            .unwrap(),
    );
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().map_while(Result::ok) {
            let cfg = cfg.clone();
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, s);
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = tls.read(&mut buf) else { return };
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                    if let Some(h) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..h]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if req.len() >= h + 4 + len {
                            let reply = http_reply(&req, h);
                            drop(tls.write_all(reply.as_bytes()));
                            drop(tls.flush());
                            tls.conn.send_close_notify();
                            drop(tls.flush());
                            return;
                        }
                    }
                }
            });
        }
    });
    (port, pem)
}

#[tokio::test(flavor = "multi_thread")]
async fn https_mcp_server_connects_with_trusted_ca_and_lists_tools() {
    let (port, pem) = tls_server();
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, pem).unwrap();
    let c = RmcpConnector::with_ca(&ca).expect("CA read");
    let session = c
        .connect(&spec(&format!("https://localhost:{port}/mcp")))
        .await
        .map_err(|e| format!("{e:?}"))
        .expect("TLS handshake");
    let tools = session.list_tools().await.expect("catalog over TLS");
    assert!(tools.iter().any(|t| t.name == "echo"), "{tools:?}");
    let r = session
        .call("echo", &json!({"text": "over https"}))
        .await
        .unwrap();
    assert!(r.text.contains("over https"), "{r:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn https_with_untrusted_certificate_is_rejected() {
    let (port, _pem) = tls_server();
    let r = RmcpConnector::default()
        .connect(&spec(&format!("https://localhost:{port}/mcp")))
        .await;
    assert!(
        matches!(r, Err(McpError::Transport)),
        "self-signed certificate without trust is rejected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn https_to_closed_port_is_transport_error_without_panic() {
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let r = RmcpConnector::default()
        .connect(&spec(&format!("https://127.0.0.1:{port}/mcp")))
        .await;
    assert!(matches!(r, Err(McpError::Transport)));
}

#[test]
fn missing_ca_file_is_an_error_not_a_silent_default() {
    let dir = tempfile::tempdir().unwrap();
    assert!(RmcpConnector::with_ca(&dir.path().join("missing.pem")).is_err());
    let bad = dir.path().join("bad.pem");
    std::fs::write(&bad, "not a certificate").unwrap();
    assert!(RmcpConnector::with_ca(&bad).is_err());
}
