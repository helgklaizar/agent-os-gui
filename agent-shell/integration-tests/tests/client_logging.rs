//! Client log against `fake-agentd` with `RUST_LOG=trace`: it has records with `request_id` and status, connection
//! state changes; it has no token, its hash, `Authorization` header, hostile strings or the actual request path.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use common::{client, http, quiet, sha256_hex, wait_state};
use core_client::ConnState;
use core_client::http::Request;
use core_client::logging::subscriber;
use serde_json::json;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;

    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// One global subscriber for the whole file, as `RUST_LOG=trace` for a process; TRACE level for all crates.
fn log() -> String {
    static SINK: OnceLock<Sink> = OnceLock::new();
    let sink = SINK.get_or_init(|| {
        let sink = Sink::default();
        let sub = subscriber(sink.clone(), EnvFilter::new("trace"));
        if let Err(e) = tracing::subscriber::set_global_default(sub) {
            panic!("subscriber not installed: {e}");
        }
        sink
    });
    String::from_utf8_lossy(&sink.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
}

const EVIL: &[&str] = &[
    "'; DROP TABLE api_token;--",
    "<script>alert(1)</script>",
    "../../etc/passwd",
    "\u{202e}rtl-override",
    "HOSTILE-STRING-NAME",
    "${jndi:ldap://x}",
];

fn assert_no_secrets(log: &str, token: &str) {
    let hash = sha256_hex(token);
    assert!(!log.contains(token), "token value in the log");
    assert!(!log.contains(&hash), "token hash in the log");
    assert!(
        !log.to_lowercase().contains("authorization"),
        "Authorization header in the log"
    );
    assert!(!log.contains("Bearer"), "Bearer scheme in the log");
    for e in EVIL {
        assert!(!log.contains(e), "hostile string {e:?} in the log");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn client_log_has_request_id_and_status_but_no_secrets_at_trace() {
    // the subscriber is installed before any request
    let _ = log();
    let d = quiet().await;
    let token = d.admin_token().to_string();
    let c = http(&d, Some(&token));
    let rid = "0198a2b4-7c3e-7d11-9a55-3f1c2b9d4e61";
    let ok = c
        .send(Request::get("/v1/settings/theme").header("X-Request-Id", rid))
        .await;
    assert_eq!(ok.map(|r| r.status).ok(), Some(200));

    let missing_rid = "0198a2b4-7c3e-7d11-9a55-3f1c2b9d4e62";
    let nope = c
        .send(
            Request::get("/v1/no-such-route-ops03?q=%27%3Cscript%3E")
                .header("X-Request-Id", missing_rid),
        )
        .await;
    assert_eq!(nope.map(|r| r.status).ok(), Some(404));

    let echo = c
        .send(Request::post(
            "/v1/_test/echo",
            json!({"client_id": "0198a2b4-7c3e-7d11-9a55-3f1c2b9d4e63", "text": EVIL.join(" | ")}),
        ))
        .await;
    assert!(echo.is_ok());
    let anon = c.send(Request::get("/v1/health").anonymous()).await;
    assert_eq!(anon.map(|r| r.status).ok(), Some(200));

    let text = log();
    let line = text
        .lines()
        .find(|l| l.contains("http.request") && l.contains(rid))
        .unwrap_or_else(|| panic!("no record of request {rid} in the log:\n{text}"));
    assert!(line.contains("status=200"), "{line}");
    assert!(line.contains("dur_ms="), "{line}");
    let err = text
        .lines()
        .find(|l| l.contains("http.error") && l.contains(missing_rid))
        .unwrap_or_else(|| panic!("no record of error {missing_rid} in the log:\n{text}"));
    assert!(err.contains("status=404"), "{err}");
    assert!(
        !text.contains("no-such-route-ops03"),
        "no actual path in the log"
    );
    assert_no_secrets(&text, &token);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_log_records_connection_state_changes_without_secrets() {
    let _ = log();
    let d = quiet().await;
    let token = d.admin_token().to_string();
    let c = client(&d);
    let rx = c.events();
    assert_eq!(wait_state(&rx, ConnState::Connected).await, Ok(()));
    assert!(c.subscribe("bar.sys").is_ok());
    d.drop_ws();
    assert_eq!(wait_state(&rx, ConnState::Reconnecting).await, Ok(()));
    d.restore();
    assert_eq!(wait_state(&rx, ConnState::Connected).await, Ok(()));

    let text = log();
    for state in ["Connected", "Reconnecting"] {
        assert!(
            text.lines()
                .any(|l| l.contains("event=\"connection.state\"") && l.contains(state)),
            "no record of state {state}:\n{text}"
        );
    }
    assert_no_secrets(&text, &token);
}
