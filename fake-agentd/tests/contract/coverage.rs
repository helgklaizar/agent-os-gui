//! Completeness. Every `openapi-draft.yaml` operation is in the mock's route tables and matches on `x-scope` and
//! `x-local-only` (the reverse direction, mock to contract, is checked by the other tests). A mismatch fails the test;
//! `KNOWN_MISSING` only shrinks.

use std::collections::BTreeSet;

use fake_agentd::admin::{
    fail_inject, files_http, modules_http, people_http, secrets_http, settings_http, tunnels_http,
};
use fake_agentd::routes::Route;
use fake_agentd::{
    bar_http, chat_http, events_http, ops_http, perm_http, routes, search_http, sess_http,
    tasks_http, tools_http, tree_http,
};

use crate::openapi::Spec;
use crate::{KNOWN_MISSING, KNOWN_UNDOCUMENTED};

fn tables() -> Vec<&'static Route> {
    let all: [&'static [Route]; 18] = [
        routes::ROUTES,
        bar_http::ROUTES,
        chat_http::ROUTES,
        events_http::ROUTES,
        ops_http::ROUTES,
        perm_http::ROUTES,
        search_http::ROUTES,
        sess_http::ROUTES,
        tasks_http::ROUTES,
        tools_http::ROUTES,
        tree_http::ROUTES,
        fail_inject::ROUTES,
        files_http::ROUTES,
        modules_http::ROUTES,
        people_http::ROUTES,
        secrets_http::ROUTES,
        settings_http::ROUTES,
        tunnels_http::ROUTES,
    ];
    all.iter().flat_map(|t| t.iter()).collect()
}

/// `/health` and `/ws` are served by the server outside the route tables (anonymous health, upgrade).
const OUTSIDE_TABLES: &[(&str, &str)] = &[("GET", "/health"), ("GET", "/ws")];

#[test]
fn contract_every_openapi_operation_exists_in_mock() {
    let spec = Spec::load();
    let mock: BTreeSet<(String, String)> = tables()
        .iter()
        .map(|r| (r.method.to_string(), r.pattern.to_string()))
        .collect();
    let mut missing: Vec<String> = Vec::new();
    for (method, path, _) in spec.operations() {
        if OUTSIDE_TABLES.contains(&(method.as_str(), path.as_str())) {
            continue;
        }
        if !mock.contains(&(method.clone(), path.clone())) {
            missing.push(format!("{method} {path}"));
        }
    }
    let known: BTreeSet<String> = KNOWN_MISSING
        .iter()
        .map(|(m, p, _)| format!("{m} {p}"))
        .collect();
    let unexpected: Vec<&String> = missing.iter().filter(|m| !known.contains(*m)).collect();
    assert!(
        unexpected.is_empty(),
        "operations are in openapi-draft.yaml and missing from the mock: {unexpected:?}"
    );
    let stale: Vec<&String> = known.iter().filter(|k| !missing.contains(k)).collect();
    assert!(
        stale.is_empty(),
        "KNOWN_MISSING is stale, remove the entries: {stale:?}"
    );
}

#[test]
fn contract_scope_and_local_only_match_for_all_mock_routes() {
    let spec = Spec::load();
    let mut errs = Vec::new();
    for r in tables() {
        if r.pattern.starts_with("/_test/") {
            continue;
        }
        let known = KNOWN_UNDOCUMENTED
            .iter()
            .any(|(m, p, _)| *m == r.method && *p == r.pattern);
        if spec.op(r.method, r.pattern).is_none() {
            if !known {
                errs.push(format!("{} {}: not in the contract", r.method, r.pattern));
            }
            continue;
        }
        if known {
            errs.push(format!(
                "{} {}: already in the contract, remove from KNOWN_UNDOCUMENTED",
                r.method, r.pattern
            ));
        }
        if spec.scope(r.method, r.pattern) != Some(r.scope.as_str()) {
            errs.push(format!("{} {}: x-scope", r.method, r.pattern));
        }
        if spec.local_only(r.method, r.pattern) != r.local_only {
            errs.push(format!("{} {}: x-local-only", r.method, r.pattern));
        }
    }
    crate::assert_clean("mock route tables", errs);
}
