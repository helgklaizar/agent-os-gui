//! `fake-agentd` contract tests against `openapi-draft.yaml` and the `api.md` section 3 error table.
//!
//! Every mock route from the contract is called over a real socket; the code, scope, `x-local-only`, body and response headers
//! are checked against the schemas. A mismatch fails the test. Known mismatches are listed below by name, with a reason:
//! the list only shrinks (the `contract_known_drift_is_still_drift` test requires removing an entry once the document is fixed).
//! A YAML crate dependency is not allowed by the dependency policy, so the parser is custom (`yaml.rs`).

mod openapi;
mod rawhttp;
mod yaml;

mod coverage;
mod errors_limits_tests;
mod errors_tests;
mod health_fields;
mod ops_routes;
mod parser_tests;
mod routes_tests;
mod s7_resources;
mod s7_routes;
mod tools_routes;
mod tree_routes;
mod ws_frames;

use std::collections::BTreeSet;
use std::path::PathBuf;

use fake_agentd::{Config, FakeAgentd, TokenSpec};
use openapi::Spec;

/// `(error code, reason)`: the code is in `api.md`, but not in the `enum` of the `Error` schema of `openapi-draft.yaml`.
pub const KNOWN_CODE_DRIFT: &[(&str, &str)] = &[(
    "forbidden_method",
    "a method outside the allowlist of an agent/module token; the code is neither in the Error schema enum nor in the api.md section 3 table",
)];

/// `(METHOD, path, status)`: the mock answers this way, but the OpenAPI operation lacks this status.
// PATCH /settings/theme: `422 validation_failed` for an unknown theme value (api.md section 3: 422 - any fields that failed validation),
// the operation describes only `412` and `428`.
pub const KNOWN_STATUS_DRIFT: &[(&str, &str, u16)] = &[("PATCH", "/settings/theme", 422)];

/// `(METHOD, path, reason)`: the operation is in the contract and not yet implemented in the mock; the list only shrinks.
pub const KNOWN_MISSING: &[(&str, &str, &str)] = &[];

/// `(METHOD, path, reason)`: the route is in the mock and in `api.md`, but not in `openapi-draft.yaml`; the list only shrinks.
pub const KNOWN_UNDOCUMENTED: &[(&str, &str, &str)] = &[(
    "GET",
    "/observer-findings",
    "api.md section 5.4 describes it; it is neither in openapi-draft.yaml nor in the real agentd",
)];

pub const REQUEST_ID: &str = "0198a2b4-7c3e-7d11-9a55-3f1c2b9d4e60";

pub async fn mock() -> FakeAgentd {
    match FakeAgentd::start(Config::temp().no_ticker()).await {
        Ok(d) => d,
        Err(e) => panic!("fake-agentd did not start: {e}"),
    }
}

pub fn token(d: &FakeAgentd, spec: TokenSpec) -> String {
    match d.issue(spec) {
        Ok(t) => t.token,
        Err(e) => panic!("token not issued: {e}"),
    }
}

pub fn api_error_table() -> BTreeSet<(u16, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/api.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let (Ok(status), Some(code)) = (
            cells[1].parse::<u16>(),
            cells[2].strip_prefix('`').and_then(|c| c.strip_suffix('`')),
        ) else {
            continue;
        };
        out.insert((status, code.to_string()));
    }
    out
}

pub fn assert_clean(what: &str, errs: Vec<String>) {
    assert!(
        errs.is_empty(),
        "{what} differs from the contract:\n  {}",
        errs.join("\n  ")
    );
}

pub fn status_documented(spec: &Spec, method: &str, path: &str, status: u16) -> bool {
    spec.statuses(method, path).contains(&status)
        || KNOWN_STATUS_DRIFT
            .iter()
            .any(|(m, p, s)| *m == method && *p == path && *s == status)
}
