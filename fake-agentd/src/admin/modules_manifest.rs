//! Module manifest: validation as in the real core, and the mock catalog.
//! The JSON schema of the manifest is in `fake-agentd/schemas/module-manifest.json`; a test checks the catalog against it.

use serde_json::{Map, Value, json};

use crate::settings_conn::FieldErr;

pub const PLACEMENTS: [&str; 3] = ["panel", "widget", "button"];
const FIELDS: [&str; 9] = [
    "id",
    "name",
    "version",
    "entry",
    "placement",
    "permissions",
    "network",
    "fs",
    "questions",
];
const FORBIDDEN: [&str; 11] = [
    "/tokens",
    "/secrets",
    "/files",
    "/permissions",
    "/permission-requests",
    "/settings",
    "/accounts",
    "/modules",
    "/tunnels",
    "/triggers",
    "/logout",
];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Mock catalog entries: ready manifests.
pub fn catalog() -> Vec<Value> {
    vec![
        json!({"id": "weather-card", "name": "Weather", "version": "1.0.0", "entry": "index.html",
               "placement": "widget", "permissions": ["GET /quotas"], "network": ["wttr.in"], "fs": [],
               "questions": [{"key": "city", "label": "City"}]}),
        json!({"id": "notes-panel", "name": "Notes", "version": "1.2.0", "entry": "panel.html",
               "placement": "panel", "permissions": ["GET /sessions"], "network": [], "fs": ["~/notes"],
               "questions": []}),
        json!({"id": "quick-btn", "name": "Quick button", "version": "0.3.0", "entry": "button.html",
               "placement": "button", "permissions": [], "network": [], "fs": [], "questions": []}),
    ]
}

fn err(field: impl Into<String>, msg: &str) -> FieldErr {
    (field.into(), msg.to_string())
}

fn valid_id(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_domain(s: &str) -> bool {
    s.contains('.')
        && s.len() <= 253
        && !s.starts_with(['.', '-'])
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

fn valid_permission(p: &str) -> bool {
    let Some((m, path)) = p.split_once(' ') else {
        return false;
    };
    METHODS.contains(&m)
        && path.starts_with('/')
        && path.len() <= 120
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_*-".contains(c))
        && !FORBIDDEN.iter().any(|f| {
            path == *f || path.starts_with(&format!("{f}/")) || path.starts_with(&format!("{f}*"))
        })
}

fn text(obj: &Map<String, Value>, k: &str, max: usize, errs: &mut Vec<FieldErr>) {
    if !obj
        .get(k)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
    {
        errs.push(err(k, &format!("string of 1 to {max} characters")));
    }
}

fn list_of<'a>(
    obj: &'a Map<String, Value>,
    k: &str,
    errs: &mut Vec<FieldErr>,
) -> Vec<(usize, &'a str)> {
    match obj.get(k) {
        None => Vec::new(),
        Some(Value::Array(a)) if a.len() <= 32 => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.as_str().unwrap_or("")))
            .collect(),
        Some(_) => {
            errs.push(err(k, "array of strings, at most 32"));
            Vec::new()
        }
    }
}

/// Manifest check; an empty list means the manifest is fine.
pub fn validate(m: &Value) -> Vec<FieldErr> {
    let Some(obj) = m.as_object() else {
        return vec![err("manifest", "object expected")];
    };
    let mut errs = Vec::new();
    for k in obj.keys().filter(|k| !FIELDS.contains(&k.as_str())) {
        errs.push(err(k.clone(), "unknown field"));
    }
    if !obj.get("id").and_then(Value::as_str).is_some_and(valid_id) {
        errs.push(err("id", "1 to 40 characters a-z, 0-9 and \"-\""));
    }
    text(obj, "name", 80, &mut errs);
    text(obj, "version", 32, &mut errs);
    match obj.get("entry").and_then(Value::as_str) {
        Some(e)
            if !e.is_empty()
                && e.len() <= 120
                && !e.starts_with('/')
                && !e.contains(['\\', ':', '\0'])
                && !e.split('/').any(|p| p.is_empty() || p == "." || p == "..") => {}
        _ => errs.push(err(
            "entry",
            "relative path without \"..\" inside the package",
        )),
    }
    if !obj
        .get("placement")
        .and_then(Value::as_str)
        .is_some_and(|p| PLACEMENTS.contains(&p))
    {
        errs.push(err("placement", "panel, widget or button"));
    }
    for (i, p) in list_of(obj, "permissions", &mut errs) {
        if !valid_permission(p) {
            errs.push(err(
                format!("permissions[{i}]"),
                "\"METHOD /path\" outside admin scopes",
            ));
        }
    }
    for (i, d) in list_of(obj, "network", &mut errs) {
        if !valid_domain(d) {
            errs.push(err(
                format!("network[{i}]"),
                "domain without scheme and wildcards",
            ));
        }
    }
    for (i, f) in list_of(obj, "fs", &mut errs) {
        if !(f.starts_with("~/") && f.len() > 2 && !f.split('/').any(|p| p == ".." || p == ".")) {
            errs.push(err(format!("fs[{i}]"), "explicit folder like ~/folder"));
        }
    }
    if obj.get("questions").is_some_and(|q| !q.is_array()) {
        errs.push(err("questions", "array"));
    }
    errs
}
