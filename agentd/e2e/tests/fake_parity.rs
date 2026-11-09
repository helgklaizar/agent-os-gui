//! A7-13: `fake_agentd_shape_parity` — response shapes of the real core and `fake-agentd` for the same requests. Compared are
//! key sets and value types (not the values). Differences are pinned by the `KNOWN` list: a new difference, or a fixed one
//! not removed from the list, fails the test. The foreign directory (`fake-agentd/`) is not edited: the list goes into the report.

mod common;

use std::collections::BTreeSet;

use common::Env;
use core_client::client::HttpClient;
use core_client::http::Request;
use core_client::{Endpoint, Token};
use fake_agentd::{Config, FakeAgentd};
use serde_json::Value;

/// Read-only requests that both sides have.
const PATHS: &[&str] = &[
    "/health",
    "/sessions",
    "/folders",
    "/personas",
    "/accounts",
    "/permissions",
    "/settings/theme",
    "/settings/indicators",
    "/settings/about",
    "/events",
    "/modules",
    "/connections",
];

async fn get(ep: Endpoint, token: &str, path: &str) -> (u16, Value) {
    let c = HttpClient::new(ep, Some(Token::new(token.to_string())));
    let r = c.send(Request::get(&format!("/v1{path}"))).await.unwrap();
    (
        r.status,
        serde_json::from_slice(&r.body).unwrap_or(Value::Null),
    )
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

/// Shape differences: keys missing on one side and differing types. `null` against any type is not a difference
/// (the field is optional); an empty array against a non-empty one is not either (different data, not shapes).
fn diff(path: &str, real: &Value, fake: &Value, out: &mut Vec<String>) {
    match (real, fake) {
        (Value::Null, _) | (_, Value::Null) => {}
        // the `by_kind` and `by_sub` keys are data (kinds and subtypes with unread items), not shape
        (Value::Object(_), Value::Object(_)) if path.contains(".counts.by_") => {}
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys().filter(|k| !b.contains_key(*k)) {
                out.push(format!("{path}.{k} only in the core"));
            }
            for k in b.keys().filter(|k| !a.contains_key(*k)) {
                out.push(format!("{path}.{k} only in fake"));
            }
            for (k, x) in a {
                if let Some(y) = b.get(k) {
                    diff(&format!("{path}.{k}"), x, y, out);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if let (Some(x), Some(y)) = (a.first(), b.first()) {
                diff(&format!("{path}[]"), x, y, out);
            }
        }
        (a, b) if kind(a) != kind(b) => {
            out.push(format!("{path} type: core {}, fake {}", kind(a), kind(b)));
        }
        _ => {}
    }
}

/// Known differences; the list was attached to the A7-rest report.
const KNOWN: &[&str] = &[];

#[tokio::test(flavor = "multi_thread")]
async fn fake_agentd_shape_parity() {
    let e = Env::start().await;
    let fake = FakeAgentd::start(Config::temp().no_event_fixtures())
        .await
        .unwrap();
    let mut found: Vec<String> = Vec::new();
    for p in PATHS {
        let (sr, real) = get(Endpoint::Unix(e.sock.clone()), &e.admin_token, p).await;
        let (sf, f) = get(
            Endpoint::Unix(fake.socket_path().to_path_buf()),
            fake.admin_token(),
            p,
        )
        .await;
        if sr != sf {
            found.push(format!("{p} status: core {sr}, fake {sf}"));
            continue;
        }
        diff(p, &real, &f, &mut found);
    }
    for line in &found {
        eprintln!("difference: {line}");
    }
    let known: BTreeSet<String> = KNOWN.iter().map(|s| (*s).to_string()).collect();
    let now: BTreeSet<String> = found.into_iter().collect();
    let new: Vec<_> = now.difference(&known).collect();
    let gone: Vec<_> = known.difference(&now).collect();
    assert!(new.is_empty(), "new differences from fake-agentd: {new:#?}");
    assert!(gone.is_empty(), "fixed, remove from KNOWN: {gone:#?}");
}
