//! Settings page registry and document validation (`docs/reference/api.md` 5.7, 9.6). `theme` and
//! `indicators` have strict schemas, the other pages are objects; fields the core already reads are checked individually.

use serde_json::{Value, json};

use crate::FieldError;

/// Pages `/settings/{page}` and the separate `theme`, `indicators` (openapi-draft.yaml).
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
    "backup",
    "memory",
    "indicators",
    "theme",
];

/// Maximum size of a page document after merging, bytes.
pub const MAX_DOC_BYTES: usize = 256 * 1024;

#[must_use]
pub fn is_known_page(page: &str) -> bool {
    PAGES.contains(&page)
}

/// `about` is read-only (`docs/reference/api.md` 5.7).
#[must_use]
pub fn is_read_only(page: &str) -> bool {
    matches!(page, "about" | "security")
}

/// Document of a page that is not in the DB yet.
#[must_use]
pub fn default_doc(page: &str) -> Value {
    match page {
        "theme" => json!({ "theme": "auto" }),
        "indicators" => json!({ "items": crate::default_items() }),
        "memory" => json!({ "source": "local" }),
        "tunnels" => json!({ "api_external": false }),
        _ => json!({}),
    }
}

fn bad(field: &str, message: &str) -> Vec<FieldError> {
    vec![FieldError::new(field, message)]
}

fn indicator_errors(items: &Value) -> Vec<FieldError> {
    let Some(list) = items.as_array() else {
        return bad("items", "an array is expected");
    };
    let mut errs = Vec::new();
    for (i, it) in list.iter().enumerate() {
        let f = |name: &str| format!("items[{i}].{name}");
        let Some(o) = it.as_object() else {
            errs.push(FieldError::new(
                &format!("items[{i}]"),
                "an object is expected",
            ));
            continue;
        };
        if !o
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            errs.push(FieldError::new(&f("id"), "a non-empty string is required"));
        }
        if !o.get("enabled").is_some_and(Value::is_boolean) {
            errs.push(FieldError::new(&f("enabled"), "true or false is required"));
        }
        if o.get("position").and_then(Value::as_u64).is_none() {
            errs.push(FieldError::new(
                &f("position"),
                "a non-negative integer is required",
            ));
        }
        if o.get("config")
            .is_some_and(|c| !c.is_object() && !c.is_null())
        {
            errs.push(FieldError::new(&f("config"), "an object is expected"));
        }
    }
    errs
}

/// Names of fields that hold secrets; the `_ref` suffix is a reference to a secret in the store, not a secret.
const SECRET_KEY_WORDS: [&str; 8] = [
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
    !low.ends_with("_ref") && SECRET_KEY_WORDS.iter().any(|w| low.contains(w))
}

/// Secret values are not stored in a page document: a non-empty string in a field with a "secret" name is a refusal.
pub fn secret_fields(v: &Value, path: &str, errs: &mut Vec<FieldError>) {
    match v {
        Value::Object(o) => {
            for (k, val) in o {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if secret_key(k) && val.as_str().is_some_and(|s| !s.is_empty()) {
                    errs.push(FieldError::new(
                        &p,
                        "secrets are stored through PUT /secrets/{ref}; only a reference (<name>_ref) is needed here",
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

/// Unknown field error text (`validate_patch` uses it to tell apart old extra fields).
pub(crate) const UNKNOWN_FIELD: &str = "unknown field";

/// Validates a document obtained after merging with a patch. An empty list — the document is fine.
#[must_use]
pub fn validate(page: &str, doc: &Value) -> Vec<FieldError> {
    let mut errs = validate_page(page, doc);
    secret_fields(doc, "", &mut errs);
    errs
}

/// Whether the field `path` (dotted names) is in the document.
fn has_path(doc: &Value, path: &str) -> bool {
    let mut cur = doc;
    for part in path.split('.') {
        match cur.get(part) {
            Some(v) => cur = v,
            None => return false,
        }
    }
    true
}

/// Validation on `PATCH`: like [`validate`], but an "unknown field" that was already in the previous document `current`
/// (a record from before schemas were introduced) is allowed; a new extra field cannot be added.
#[must_use]
pub fn validate_patch(page: &str, merged: &Value, current: &Value) -> Vec<FieldError> {
    let mut errs = validate(page, merged);
    errs.retain(|e| e.message != UNKNOWN_FIELD || !has_path(current, &e.field));
    errs
}

fn validate_page(page: &str, doc: &Value) -> Vec<FieldError> {
    let Some(obj) = doc.as_object() else {
        return bad("", "the page document is an object");
    };
    if serde_json::to_vec(doc).map_or(usize::MAX, |b| b.len()) > MAX_DOC_BYTES {
        return bad("", "the document is larger than allowed");
    }
    match page {
        "theme" => {
            let mut errs = Vec::new();
            if !matches!(
                obj.get("theme").and_then(Value::as_str),
                Some("auto" | "dark" | "light")
            ) {
                errs.push(FieldError::new("theme", "allowed: auto, dark or light"));
            }
            for k in obj.keys().filter(|k| *k != "theme") {
                errs.push(FieldError::new(k, "unknown field"));
            }
            errs
        }
        "indicators" => obj.get("items").map_or_else(Vec::new, indicator_errors),
        _ => page_errors(page, doc, obj),
    }
}

/// Pages with a schema: first the schema file, then checks of nested `models` and `connections` entries.
fn page_errors(page: &str, doc: &Value, obj: &serde_json::Map<String, Value>) -> Vec<FieldError> {
    let mut errs = Vec::new();
    if let Some(schema) = crate::settings_json_schema::schema_of(page) {
        crate::settings_json_schema::check(schema, doc, "", &mut errs);
        if !errs.is_empty() {
            return errs;
        }
    }
    match page {
        "connections" => crate::validate_connections(obj),
        "models" => crate::settings_models::validate(obj),
        "backup" => match obj.get("last_restore_test_at") {
            Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 0) => Vec::new(),
            Some(Value::Null) | None => Vec::new(),
            Some(_) => bad(
                "last_restore_test_at",
                "unix seconds (an integer) or null are required",
            ),
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rules() {
        assert!(validate("theme", &json!({"theme":"dark"})).is_empty());
        assert!(!validate("theme", &json!({"theme":"red"})).is_empty());
        assert!(!validate("theme", &json!({})).is_empty());
        assert!(!validate("theme", &json!({"theme":"dark","x":1})).is_empty());
        assert!(!validate("account", &json!([])).is_empty());
        assert!(validate("memory", &json!({"source":"both"})).is_empty());
        assert!(!validate("memory", &json!({"source":"cloud"})).is_empty());
        assert!(!validate("memory", &json!({"source":1})).is_empty());
        assert!(validate("tunnels", &json!({"api_external":true})).is_empty());
        assert!(!validate("tunnels", &json!({"api_external":"yes"})).is_empty());
        assert!(validate("permissions", &json!({"ask_ttl_min":10,"rules_rev":3})).is_empty());
        assert!(!validate("permissions", &json!({"ask_ttl_min":0})).is_empty());
        assert!(!validate("permissions", &json!({"ask_ttl_min":1441})).is_empty());
        assert!(!validate("permissions", &json!({"x":1})).is_empty());
        let good = json!({"items":[{"id":"cpu","enabled":true,"position":0}]});
        assert!(validate("indicators", &good).is_empty());
        let bad_items = json!({"items":[{"id":"","enabled":1,"position":-1}]});
        assert_eq!(validate("indicators", &bad_items).len(), 3);
        assert!(
            PAGES
                .iter()
                .all(|p| validate(p, &default_doc(p)).is_empty())
        );
    }
}
