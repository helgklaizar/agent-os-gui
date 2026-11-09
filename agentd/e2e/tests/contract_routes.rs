//! A7-13 / A6-14: `contract_all_routes_covered`: every route from `openapi-draft.yaml` exists in the core (the reply is not "no such
//! path"), and the list of not-yet-implemented routes is fixed explicitly: a new route without an implementation, or an implemented one
//! not removed from the list, fails the test.

mod common;
mod openapi;

use std::collections::BTreeSet;

use common::Env;
use openapi::YAML;
use serde_json::{Value, json};

/// "method path" pairs from OpenAPI.
fn spec_routes() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    for line in YAML.lines() {
        if line.starts_with("components:") {
            break;
        }
        if let Some(p) = line.strip_prefix("  /").and_then(|p| p.strip_suffix(':')) {
            path = Some(format!("/{p}"));
        } else if let (Some(p), Some(m)) = (
            path.as_ref(),
            line.strip_prefix("    ")
                .and_then(|m| m.strip_suffix(':'))
                .filter(|m| ["get", "post", "put", "patch", "delete"].contains(m)),
        ) {
            out.push((m.to_uppercase(), p.clone()));
        }
    }
    out
}

fn fill(path: &str) -> String {
    let mut out = String::new();
    for part in path.split('/') {
        if part.is_empty() {
            continue;
        }
        out.push('/');
        if part.starts_with('{') {
            out.push_str(match part {
                "{ref}" => "contract.test",
                "{module_id}" => "contract-module",
                "{page}" => "theme",
                "{scope}" => "global",
                "{scope_id}" => "0",
                "{action}" => "read",
                _ => "999999",
            });
        } else {
            out.push_str(part);
        }
    }
    out
}

/// Routes the core does not implement yet (`404 not_implemented`); there are none now.
const NOT_IMPLEMENTED: &[&str] = &[];

#[tokio::test(flavor = "multi_thread")]
async fn contract_all_routes_covered() {
    let e = Env::start().await;
    let routes = spec_routes();
    assert!(routes.len() > 60, "parsed {} operations", routes.len());
    let mut missing: BTreeSet<String> = BTreeSet::new();
    let mut gaps: BTreeSet<String> = BTreeSet::new();
    for (method, path) in &routes {
        if path == "/ws" || path.starts_with("/_test") {
            continue;
        }
        let url = fill(path);
        let body = json!({});
        let (st, v): (u16, Value) = match method.as_str() {
            "GET" => e.admin.get(&url).await,
            "POST" => e.admin.post(&url, body).await,
            "PUT" => e.admin.put(&url, body).await,
            "PATCH" => e.admin.patch(&url, body).await,
            _ => e.admin.delete(&url).await,
        };
        let key = format!("{method} {path}");
        if st == 404 && v["error"]["details"]["reason"] == "not_implemented" {
            gaps.insert(key);
        } else if st == 404
            && v["error"]["code"] == "not_found"
            && v["error"]["details"].is_null()
            && !path.contains('{')
        {
            missing.insert(key);
        } else if st == 405 {
            missing.insert(format!("{key} (method not allowed)"));
        }
    }
    assert!(
        missing.is_empty(),
        "OpenAPI routes missing from the core: {missing:?}"
    );
    let expected: BTreeSet<String> = NOT_IMPLEMENTED.iter().map(|s| (*s).to_string()).collect();
    let unexpected: Vec<&String> = gaps.difference(&expected).collect();
    let closed: Vec<&String> = expected.difference(&gaps).collect();
    assert!(
        unexpected.is_empty(),
        "new unimplemented routes: {unexpected:?}"
    );
    assert!(
        closed.is_empty(),
        "already implemented, remove from NOT_IMPLEMENTED: {closed:?}"
    );
}
