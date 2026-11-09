//! OPS-03 (`request_id_roundtrip`, observability.md 3): a core journal entry for a request carries the same `request_id` as
//! the response header; the entry has no token value, no `Authorization` header and no actual path.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{TOKEN_READ, config, start, unix_req};

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
async fn request_id_reaches_the_core_log_without_secrets() {
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
    // Request without a token to a protected route: 401 rejection, client id preserved.
    let resp = unix_req(server.socket_path(), "GET", "/v1/sessions/987654", None).await;
    assert_eq!(resp.status, 401);
    let rid = resp
        .header("x-request-id")
        .expect("header is always present")
        .to_string();
    // Successful request with a token: debug-level entry.
    let ok = unix_req(server.socket_path(), "GET", "/v1/health", Some(TOKEN_READ)).await;
    assert_eq!(ok.status, 200);
    let ok_rid = ok.header("x-request-id").expect("header").to_string();
    server.shutdown().await;

    let log = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    for (rid, status) in [(&rid, "status=401"), (&ok_rid, "status=200")] {
        let line = log
            .lines()
            .find(|l| l.contains("event=\"http\"") && l.contains(rid.as_str()))
            .unwrap_or_else(|| panic!("no entry for request {rid} in the journal: {log}"));
        assert!(line.contains(status), "{line}");
        assert!(line.contains("dur_ms="), "{line}");
    }
    assert!(!log.contains(TOKEN_READ), "no token value in the journal");
    assert!(
        !log.to_lowercase().contains("authorization"),
        "no Authorization header in the journal"
    );
    assert!(
        !log.contains("987654"),
        "no actual path in the journal: {log}"
    );
}
