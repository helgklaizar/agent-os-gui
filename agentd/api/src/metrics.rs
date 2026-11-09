//! Core metrics (docs/reference/api.md 5.10): counters and a histogram without user data.
//! The route label is the table template (`/sessions/{id}`), not the actual path.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// Upper bounds of the latency buckets, ms.
const BUCKETS_MS: [u64; 8] = [5, 10, 25, 50, 100, 250, 500, 1000];

#[derive(Default)]
struct Duration {
    buckets: [u64; 8],
    count: u64,
    sum_ms: u64,
}

#[derive(Default)]
struct Inner {
    requests: BTreeMap<(String, u16), u64>,
    errors: BTreeMap<String, u64>,
    durations: BTreeMap<String, Duration>,
}

#[derive(Default)]
pub(crate) struct Metrics {
    inner: Mutex<Inner>,
    pub ws_connections: AtomicI64,
    pub ws_total: AtomicU64,
}

impl Metrics {
    pub(crate) fn record(&self, route: &str, status: u16, error_code: Option<&str>, dur_ms: u64) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *m.requests.entry((route.to_string(), status)).or_default() += 1;
        if let Some(code) = error_code {
            *m.errors.entry(code.to_string()).or_default() += 1;
        }
        let d = m.durations.entry(route.to_string()).or_default();
        d.count += 1;
        d.sum_ms += dur_ms;
        for (i, b) in BUCKETS_MS.iter().enumerate() {
            if dur_ms <= *b {
                d.buckets[i] += 1;
            }
        }
    }

    /// Text exposition; `queue_len` and `user_version` come from the DB.
    pub(crate) fn render(&self, queue_len: usize, user_version: Option<i64>) -> String {
        let m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = String::new();
        out.push_str("# TYPE http_requests_total counter\n");
        for ((route, status), n) in &m.requests {
            writeln!(
                out,
                "http_requests_total{{route=\"{route}\",status=\"{status}\"}} {n}"
            )
            .ok();
        }
        out.push_str("# TYPE http_errors_total counter\n");
        for (code, n) in &m.errors {
            writeln!(out, "http_errors_total{{code=\"{code}\"}} {n}").ok();
        }
        out.push_str("# TYPE http_request_duration_ms histogram\n");
        for (route, d) in &m.durations {
            for (i, b) in BUCKETS_MS.iter().enumerate() {
                writeln!(
                    out,
                    "http_request_duration_ms_bucket{{route=\"{route}\",le=\"{b}\"}} {}",
                    d.buckets[i]
                )
                .ok();
            }
            writeln!(
                out,
                "http_request_duration_ms_bucket{{route=\"{route}\",le=\"+Inf\"}} {}",
                d.count
            )
            .ok();
            writeln!(
                out,
                "http_request_duration_ms_sum{{route=\"{route}\"}} {}",
                d.sum_ms
            )
            .ok();
            writeln!(
                out,
                "http_request_duration_ms_count{{route=\"{route}\"}} {}",
                d.count
            )
            .ok();
        }
        writeln!(
            out,
            "# TYPE ws_connections gauge\nws_connections {}",
            self.ws_connections.load(Ordering::SeqCst)
        )
        .ok();
        writeln!(
            out,
            "# TYPE ws_connections_total counter\nws_connections_total {}",
            self.ws_total.load(Ordering::SeqCst)
        )
        .ok();
        writeln!(
            out,
            "# TYPE agentd_db_write_queue_len gauge\nagentd_db_write_queue_len {queue_len}"
        )
        .ok();
        if let Some(v) = user_version {
            writeln!(
                out,
                "# TYPE agentd_user_version gauge\nagentd_user_version {v}"
            )
            .ok();
        }
        out
    }
}
