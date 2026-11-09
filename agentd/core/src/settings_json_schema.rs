//! Settings page schemas: the files `agentd/api/schemas/settings/<page>.json` are embedded in the core and validate the document
//! after a patch merge. A small set of JSON Schema keywords is supported: `type`, `enum`, `minimum`, `maximum`,
//! `maxLength`, `properties`, `additionalProperties: false`, `x-format: hhmm` (time `HH:MM`).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::FieldError;

const FILES: [(&str, &str); 11] = [
    (
        "account",
        include_str!("../../api/schemas/settings/account.json"),
    ),
    (
        "about",
        include_str!("../../api/schemas/settings/about.json"),
    ),
    (
        "personas",
        include_str!("../../api/schemas/settings/personas.json"),
    ),
    (
        "models",
        include_str!("../../api/schemas/settings/models.json"),
    ),
    (
        "connections",
        include_str!("../../api/schemas/settings/connections.json"),
    ),
    (
        "permissions",
        include_str!("../../api/schemas/settings/permissions.json"),
    ),
    (
        "security",
        include_str!("../../api/schemas/settings/security.json"),
    ),
    (
        "schedule",
        include_str!("../../api/schemas/settings/schedule.json"),
    ),
    (
        "modules",
        include_str!("../../api/schemas/settings/modules.json"),
    ),
    (
        "tunnels",
        include_str!("../../api/schemas/settings/tunnels.json"),
    ),
    (
        "memory",
        include_str!("../../api/schemas/settings/memory.json"),
    ),
];

fn schemas() -> &'static HashMap<&'static str, Value> {
    static CELL: OnceLock<HashMap<&'static str, Value>> = OnceLock::new();
    CELL.get_or_init(|| {
        FILES
            .iter()
            .filter_map(|(page, text)| Some((*page, serde_json::from_str(text).ok()?)))
            .collect()
    })
}

/// The page schema, if there is one.
pub(crate) fn schema_of(page: &str) -> Option<&'static Value> {
    schemas().get(page)
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => false,
    }
}

fn hhmm(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit)
        && s[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && s[3..].parse::<u8>().is_ok_and(|m| m < 60)
}

fn describe(schema: &Value) -> String {
    if let Some(e) = schema["enum"].as_array() {
        let items: Vec<String> = e
            .iter()
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
            .collect();
        return format!("allowed: {}", items.join(", "));
    }
    if schema["x-format"] == "hhmm" {
        return "time HH:MM".into();
    }
    let ty = schema["type"].as_str().unwrap_or("value");
    let name = match ty {
        "string" => "string",
        "boolean" => "true or false",
        "integer" => "integer",
        "number" => "number",
        "object" => "object",
        "array" => "array",
        other => other,
    };
    match (schema["minimum"].as_f64(), schema["maximum"].as_f64()) {
        (Some(lo), Some(hi)) => format!("{name} from {lo} to {hi}"),
        (Some(lo), None) => format!("{name} at least {lo}"),
        _ => name.to_string(),
    }
}

/// Validates a value against the schema, errors carry the dotted field path.
pub(crate) fn check(schema: &Value, v: &Value, path: &str, out: &mut Vec<FieldError>) {
    if let Some(e) = schema["enum"].as_array()
        && !e.contains(v)
    {
        out.push(FieldError::new(path, &describe(schema)));
        return;
    }
    if let Some(t) = schema["type"].as_str()
        && !type_ok(t, v)
    {
        out.push(FieldError::new(path, &describe(schema)));
        return;
    }
    if let Some(n) = v.as_f64() {
        let low = schema["minimum"].as_f64().is_some_and(|lo| n < lo);
        let high = schema["maximum"].as_f64().is_some_and(|hi| n > hi);
        if low || high {
            out.push(FieldError::new(path, &describe(schema)));
            return;
        }
    }
    if let Some(s) = v.as_str() {
        let long = schema["maxLength"]
            .as_u64()
            .is_some_and(|m| s.chars().count() as u64 > m);
        if long {
            out.push(FieldError::new(
                path,
                &format!("at most {} characters", schema["maxLength"]),
            ));
        }
        if schema["x-format"] == "hhmm" && !hhmm(s) {
            out.push(FieldError::new(path, &describe(schema)));
        }
    }
    if let (Some(props), Some(o)) = (schema["properties"].as_object(), v.as_object()) {
        for (k, val) in o {
            match props.get(k) {
                Some(sub) => check(sub, val, &join(path, k), out),
                None if schema["additionalProperties"] == false => {
                    out.push(FieldError::new(&join(path, k), "unknown field"));
                }
                None => {}
            }
        }
    }
}
