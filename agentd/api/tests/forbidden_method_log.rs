//! A1-05: an `allowlist` rejection is written to the `tracing` journal (token id, kind, route template), without the token value and actual path.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{TOKEN_MODULE, config, start, unix_req};

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn forbidden_method_is_logged_without_secrets() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .without_time()
        .with_writer(move || LogWriter(sink.clone()))
        .finish();
    tracing::subscriber::set_global_default(sub).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let resp = unix_req(
        server.socket_path(),
        "POST",
        "/v1/sessions/777/messages",
        Some(TOKEN_MODULE),
    )
    .await;
    assert_eq!(resp.status, 403);
    server.shutdown().await;

    let log = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    let line = log
        .lines()
        .find(|l| l.contains("forbidden_method"))
        .unwrap_or_else(|| panic!("no rejection entry in the journal: {log}"));
    assert!(line.contains("token_id=4"), "{line}");
    assert!(
        line.contains("/sessions/{id}/messages"),
        "route template: {line}"
    );
    assert!(!log.contains(TOKEN_MODULE), "no token value in the journal");
    assert!(
        !log.contains("/sessions/777"),
        "no actual path in the journal"
    );
}
