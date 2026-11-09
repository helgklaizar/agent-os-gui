//! Settings pages (api.md section 5.7): registry, defaults, merge patch and document validation.
//! The rules for `models`, `connections`, `permissions`, `tunnels`, `memory` repeat the real core; for `personas`, `account`
//! and `schedule` the real core checks nothing, here it is strict per the spec (a field beyond the schema is an "unknown field").

use serde_json::{Map, Value, json};

use crate::settings_conn::{self, FieldErr};

/// Pages `/settings/{page}`; `theme` and `indicators` are served by their own routes.
pub const PAGES: &[&str] = &[
    "account",
    "about",
    "personas",
    "models",
    "connections",
    "permissions",
    "security",
    "schedule",
    "modules",
    "tunnels",
    "memory",
];

/// Page document size after merging, in bytes.
pub const MAX_DOC_BYTES: usize = 256 * 1024;

pub fn is_known(page: &str) -> bool {
    PAGES.contains(&page)
}

/// `about` and `security` are computed and not written.
pub fn is_read_only(page: &str) -> bool {
    matches!(page, "about" | "security")
}

pub fn default_doc(page: &str) -> Value {
    match page {
        "personas" => json!({ "balance": { "fallback": "local" } }),
        "memory" => json!({ "source": "local" }),
        "tunnels" => json!({ "api_external": false }),
        _ => json!({}),
    }
}

/// JSON merge patch (RFC 7386): `null` removes a key, objects merge, everything else is replaced.
pub fn merge_patch(target: &Value, patch: &Value) -> Value {
    let Value::Object(p) = patch else {
        return patch.clone();
    };
    let mut out: Map<String, Value> = match target {
        Value::Object(t) => t.clone(),
        _ => Map::new(),
    };
    for (k, v) in p {
        if v.is_null() {
            out.remove(k);
        } else {
            let merged = merge_patch(out.get(k).unwrap_or(&Value::Null), v);
            out.insert(k.clone(), merged);
        }
    }
    Value::Object(out)
}

fn err(field: impl Into<String>, msg: &str) -> FieldErr {
    (field.into(), msg.to_string())
}

const SECRET_WORDS: [&str; 8] = [
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "private_key",
    "authorization",
];

fn secret_key(k: &str) -> bool {
    let low = k.to_lowercase();
    !low.ends_with("_ref") && SECRET_WORDS.iter().any(|w| low.contains(w))
}

/// Secret values are not stored in the document: a non-empty string in a field with a "secret" name is a refusal.
pub fn secret_fields(v: &Value, path: &str, errs: &mut Vec<FieldErr>) {
    match v {
        Value::Object(o) => {
            for (k, val) in o {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if secret_key(k) && val.as_str().is_some_and(|s| !s.is_empty()) {
                    errs.push(err(
                        p,
                        "secrets are stored via PUT /secrets/{ref}; only a reference (<name>_ref) is needed here",
                    ));
                } else {
                    secret_fields(val, &p, errs);
                }
            }
        }
        Value::Array(a) => {
            for (i, val) in a.iter().enumerate() {
                secret_fields(val, &format!("{path}[{i}]"), errs);
            }
        }
        _ => {}
    }
}

/// Type of a simple page field.
#[derive(Clone, Copy)]
enum K {
    Str(usize),
    Bool,
    Enum(&'static [&'static str]),
    Int(i64, i64),
    IntOrNull,
    Time,
}

fn valid_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && s[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && s[3..].parse::<u8>().is_ok_and(|m| m < 60)
        && b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit)
}

fn ok_kind(v: &Value, k: K) -> bool {
    match k {
        K::Str(max) => v.as_str().is_some_and(|s| s.chars().count() <= max),
        K::Bool => v.is_boolean(),
        K::Enum(e) => v.as_str().is_some_and(|s| e.contains(&s)),
        K::Int(lo, hi) => v.as_i64().is_some_and(|n| (lo..=hi).contains(&n)),
        K::IntOrNull => v.is_null() || v.as_i64().is_some_and(|n| n > 0),
        K::Time => v.as_str().is_some_and(valid_time),
    }
}

/// Check an object against the field table; with `strict` a field outside the table is rejected.
fn fields(o: &Value, path: &str, spec: &[(&str, K)], strict: bool, out: &mut Vec<FieldErr>) {
    let Some(m) = o.as_object() else {
        out.push(err(path, "object expected"));
        return;
    };
    for (k, v) in m {
        let p = if path.is_empty() {
            k.clone()
        } else {
            format!("{path}.{k}")
        };
        match spec.iter().find(|(n, _)| n == k) {
            Some((_, kind)) if !ok_kind(v, *kind) => out.push(err(p, "invalid value")),
            Some(_) => {}
            None if strict => out.push(err(p, "unknown field")),
            None => {}
        }
    }
}

fn sub<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    m.get(key).filter(|v| !v.is_null())
}

const BALANCE: &[(&str, K)] = &[
    ("fallback", K::Enum(&["local", "other", "wait"])),
    ("warn_pct", K::Int(1, 100)),
    ("priority_persona_id", K::IntOrNull),
    ("distribution", K::Enum(&["even", "priority", "cheapest"])),
    ("max_parallel", K::Int(1, 16)),
    ("unified_feed", K::Bool),
    ("shared_memory", K::Bool),
    ("quiet_from", K::Time),
    ("quiet_to", K::Time),
];

fn personas(m: &Map<String, Value>) -> Vec<FieldErr> {
    let mut out = Vec::new();
    for k in m
        .keys()
        .filter(|k| !["balance", "roles"].contains(&k.as_str()))
    {
        out.push(err(k.clone(), "unknown field"));
    }
    if let Some(b) = sub(m, "balance") {
        fields(b, "balance", BALANCE, true, &mut out);
    }
    if let Some(r) = sub(m, "roles") {
        let roles: Vec<(&str, K)> = ["chat", "observer", "code", "feed"]
            .iter()
            .map(|n| (*n, K::IntOrNull))
            .collect();
        fields(r, "roles", &roles, true, &mut out);
    }
    out
}

fn account(m: &Map<String, Value>) -> Vec<FieldErr> {
    let mut out = Vec::new();
    let top = Value::Object(m.clone());
    let spec = [("name", K::Str(80)), ("language", K::Enum(&["ru", "en"]))];
    fields(&top, "", &spec, false, &mut out);
    if let Some(s) = sub(m, "security") {
        let sec = [
            ("observer", K::Bool),
            ("telemetry", K::Bool),
            ("injection_guard", K::Bool),
        ];
        fields(s, "security", &sec, true, &mut out);
    }
    out
}

fn schedule(m: &Map<String, Value>) -> Vec<FieldErr> {
    let mut out = Vec::new();
    for k in m
        .keys()
        .filter(|k| !["summary", "backup"].contains(&k.as_str()))
    {
        out.push(err(k.clone(), "unknown field"));
    }
    if let Some(s) = sub(m, "summary") {
        let spec = [("enabled", K::Bool), ("time", K::Time)];
        fields(s, "summary", &spec, true, &mut out);
    }
    if let Some(b) = sub(m, "backup") {
        let spec = [("enabled", K::Bool), ("time", K::Time)];
        let mut plain = b.clone();
        let recipient = plain
            .as_object_mut()
            .and_then(|o| o.remove("recipient"))
            .filter(|v| !v.is_null());
        fields(&plain, "backup", &spec, true, &mut out);
        if let Some(r) = recipient {
            let spec = [("primary", K::Str(200)), ("recovery", K::Str(200))];
            fields(&r, "backup.recipient", &spec, true, &mut out);
        }
    }
    out
}

fn permissions(m: &Map<String, Value>) -> Vec<FieldErr> {
    let mut out = Vec::new();
    for (k, v) in m {
        match k.as_str() {
            "ask_ttl_min" if v.as_u64().is_some_and(|n| (1..=1440).contains(&n)) => {}
            "ask_ttl_min" => out.push(err(k.clone(), "an integer from 1 to 1440 is required")),
            "rules_rev" if v.as_u64().is_some() => {}
            "rules_rev" => out.push(err(k.clone(), "a non-negative integer is required")),
            _ => out.push(err(k.clone(), "unknown field")),
        }
    }
    out
}

/// Document check after merging; an empty list means the document is fine.
pub fn validate(page: &str, doc: &Value) -> Vec<FieldErr> {
    let Some(m) = doc.as_object() else {
        return vec![err("", "the page document must be an object")];
    };
    if serde_json::to_vec(doc).map_or(usize::MAX, |b| b.len()) > MAX_DOC_BYTES {
        return vec![err("", "the document is larger than the allowed size")];
    }
    let mut errs = match page {
        "personas" => personas(m),
        "account" => account(m),
        "schedule" => schedule(m),
        "permissions" => permissions(m),
        "models" => settings_conn::models(m),
        "connections" => settings_conn::connections(m),
        "tunnels" => match m.get("api_external") {
            Some(Value::Bool(_)) | None => Vec::new(),
            Some(_) => vec![err("api_external", "true or false is required")],
        },
        "memory" => match m.get("source").and_then(Value::as_str) {
            None if !m.contains_key("source") => Vec::new(),
            Some("local" | "mcp_memory" | "both") => Vec::new(),
            _ => vec![err("source", "allowed: local, mcp_memory or both")],
        },
        _ => Vec::new(),
    };
    secret_fields(doc, "", &mut errs);
    errs
}
