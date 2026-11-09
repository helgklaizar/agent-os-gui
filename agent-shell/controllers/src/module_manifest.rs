//! Module manifest (ADR-0004): validation before installation. The rules are the same as the core's and are described by the schema
//! `fake-agentd/schemas/module-manifest.json`: required `id`, `name`, `version`, `entry`, `placement`;
//! an unknown field, invalid placement, escaping the package, or an `admin` scope in methods is rejected.

use serde_json::{Map, Value};

pub const FIELDS: [&str; 9] = [
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
pub const PLACEMENTS: [&str; 3] = ["panel", "widget", "button"];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// API scopes a module cannot obtain.
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
const MAX_LIST: usize = 32;

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

fn text_ok(o: &Map<String, Value>, k: &str, max: usize) -> bool {
    o.get(k)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
}

fn strings<'a>(o: &'a Map<String, Value>, k: &str, bad: &mut Vec<String>) -> Vec<(usize, &'a str)> {
    match o.get(k) {
        None => Vec::new(),
        Some(Value::Array(a)) if a.len() <= MAX_LIST => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.as_str().unwrap_or("")))
            .collect(),
        Some(_) => {
            bad.push(k.to_string());
            Vec::new()
        }
    }
}

/// Manifest fields that failed validation; an empty list means the manifest is valid.
pub fn validate(m: &Value) -> Vec<String> {
    let Some(o) = m.as_object() else {
        return vec!["manifest".to_string()];
    };
    let mut bad: Vec<String> = o
        .keys()
        .filter(|k| !FIELDS.contains(&k.as_str()))
        .cloned()
        .collect();
    if !o.get("id").and_then(Value::as_str).is_some_and(valid_id) {
        bad.push("id".into());
    }
    for (k, max) in [("name", 80), ("version", 32)] {
        if !text_ok(o, k, max) {
            bad.push(k.into());
        }
    }
    let entry_ok = o.get("entry").and_then(Value::as_str).is_some_and(|e| {
        !e.is_empty()
            && e.len() <= 120
            && !e.starts_with('/')
            && !e.contains(['\\', ':', '\0'])
            && !e.split('/').any(|p| p.is_empty() || p == "." || p == "..")
    });
    if !entry_ok {
        bad.push("entry".into());
    }
    if !o
        .get("placement")
        .and_then(Value::as_str)
        .is_some_and(|p| PLACEMENTS.contains(&p))
    {
        bad.push("placement".into());
    }
    for (i, p) in strings(o, "permissions", &mut bad) {
        if !valid_permission(p) {
            bad.push(format!("permissions[{i}]"));
        }
    }
    for (i, d) in strings(o, "network", &mut bad) {
        if !valid_domain(d) {
            bad.push(format!("network[{i}]"));
        }
    }
    for (i, f) in strings(o, "fs", &mut bad) {
        let ok = f.starts_with("~/") && f.len() > 2 && !f.split('/').any(|p| p == ".." || p == ".");
        if !ok {
            bad.push(format!("fs[{i}]"));
        }
    }
    if o.get("questions").is_some_and(|q| !q.is_array()) {
        bad.push("questions".into());
    }
    bad
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good() -> Value {
        json!({"id": "notes-panel", "name": "N", "version": "1.0", "entry": "index.html", "placement": "panel"})
    }

    #[test]
    fn good_manifest_passes() {
        assert!(validate(&good()).is_empty());
    }

    #[test]
    fn each_rule_rejects() {
        let cases = [
            ("placement", json!({"placement": "window"})),
            ("id", json!({"id": "Bad_Id"})),
            ("entry", json!({"entry": "../x.html"})),
            ("entry", json!({"entry": "/abs.html"})),
            ("extra", json!({"extra": 1})),
            ("permissions[0]", json!({"permissions": ["GET /tokens"]})),
            ("permissions[0]", json!({"permissions": ["FETCH /x"]})),
            ("network[0]", json!({"network": ["https://x.test"]})),
            ("fs[0]", json!({"fs": ["/etc"]})),
            ("questions", json!({"questions": "x"})),
        ];
        for (field, patch) in cases {
            let mut m = good();
            if let (Value::Object(a), Value::Object(b)) = (&mut m, patch) {
                a.extend(b);
            }
            assert_eq!(validate(&m), vec![field.to_string()], "{field}");
        }
    }

    #[test]
    fn missing_id_and_non_object_rejected() {
        let mut m = good();
        if let Value::Object(o) = &mut m {
            o.remove("id");
        }
        assert_eq!(validate(&m), vec!["id".to_string()]);
        assert_eq!(validate(&json!([])), vec!["manifest".to_string()]);
    }
}
