//! Secret masking in log lines.

use std::io::Write;
use std::sync::{Arc, Mutex};

use core_client::logging::{REDACTED, ScrubMake, scrub, subscriber};
use tracing_subscriber::EnvFilter;

/// A token in exactly the core generator's format: `agt_` and 43 base64url characters (synthetic value).
fn token() -> String {
    format!("agt_{}", &"Qa1-Zb2_".repeat(6)[..43])
}

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

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

#[test]
fn scrub_masks_bearer_header_value() {
    let t = token();
    for line in [
        format!("send Authorization: Bearer {t} ok"),
        format!("send authorization=\"Bearer {t}\" ok"),
        format!("headers {{\"Authorization\": \"Bearer {t}\"}}"),
        format!("BEARER {t}"),
    ] {
        let out = scrub(&line);
        assert!(!out.contains(&t), "token left: {out}");
        assert!(out.contains(REDACTED), "no marker: {out}");
    }
}

#[test]
fn scrub_masks_ws_subprotocol_query_and_generator_format() {
    let t = token();
    let cases = [
        format!("Sec-WebSocket-Protocol: agentd.v1, bearer.{t}"),
        format!("GET /v1/ws?token={t}&x=1"),
        format!("url=/v1/events?access_token={t}"),
        "secret_ref=my-secret-ref-value done".to_string(),
        "password=hunter2 done".to_string(),
        format!("value {t} in text without a marker"),
    ];
    for line in cases {
        let out = scrub(&line);
        for leaked in [&t[..], "my-secret-ref-value", "hunter2"] {
            assert!(!out.contains(leaked), "{leaked} left: {out}");
        }
        assert!(out.contains(REDACTED), "no marker: {out}");
    }
    let out = scrub(&format!("GET /v1/ws?token={t}&x=1"));
    assert!(out.contains("&x=1"), "rest of the query intact: {out}");
}

#[test]
fn scrub_keeps_innocent_text_and_is_idempotent() {
    for line in [
        "ws.connected scope=\"connection\" status=200 dur_ms=3",
        "token rejected with code 401",
        "request_id=0198a2b4-7c3e-7d11-9a55-3f1c2b9d4e60",
        "agt_ short prefix without a value",
        "",
    ] {
        assert_eq!(scrub(line), line, "harmless line changed");
    }
    let t = token();
    let once = scrub(&format!("Bearer {t}"));
    assert_eq!(scrub(&once), once);
}

#[test]
fn scrub_handles_non_ascii_and_edges_without_panic() {
    let t = token();
    for line in [
        "Bearer".to_string(),
        "Bearer ".to_string(),
        "token=".to_string(),
        format!("test Bearer {t} end ÉÖ"),
        format!("token={t}"),
        "authorization=\"".to_string(),
    ] {
        let out = scrub(&line);
        assert!(!out.contains(&t), "{out}");
    }
}

#[test]
fn product_subscriber_masks_what_the_code_logs() {
    let sink = Sink::default();
    let sub = subscriber(sink.clone(), EnvFilter::new("trace"));
    let t = token();
    tracing::subscriber::with_default(sub, || {
        tracing::info!(note = %format!("Authorization: Bearer {t}"), "accidental leak");
        tracing::debug!("ws bearer.{t}");
    });
    let log = sink.text();
    assert!(log.contains("accidental leak"), "{log}");
    assert!(!log.contains(&t), "token in the log: {log}");
    assert!(log.contains(REDACTED), "{log}");
}

#[test]
fn scrub_make_wraps_any_writer_factory() {
    let sink = Sink::default();
    let make = ScrubMake({
        let s = sink.clone();
        move || s.clone()
    });
    let sub = tracing_subscriber::fmt()
        .with_writer(make)
        .with_ansi(false)
        .finish();
    let t = token();
    tracing::subscriber::with_default(sub, || tracing::warn!("Bearer {t}"));
    assert!(!sink.text().contains(&t), "{}", sink.text());
}
