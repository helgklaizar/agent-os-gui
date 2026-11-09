//! Editing Settings pages from the browser (`PATCH /v1/settings/{page}` with `If-Match`, JSON merge patch, docs/reference/api.md section 5.7).
//! Fields are built from the page document; secrets are not given to the client and not written from the browser, pages
//! closed to the web token (`models`, `connections`, `permissions`, `security`, `tunnels`, `modules`, `about`)
//! stay view-only.

use serde_json::{Map, Value};

/// Pages the web token of scope `decide` edits over TCP (the same set as in `agentd/api/src/access.rs`);
/// `theme` is available to a `chat` token.
pub const WEB_EDITABLE: [&str; 6] = [
    "account",
    "personas",
    "schedule",
    "memory",
    "indicators",
    "theme",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldErr {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Number,
    Text,
    /// Object or array: edited as JSON text.
    Json,
    /// One of the allowed options.
    Choice(Vec<&'static str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub kind: Kind,
    /// Current value as input field text (`true`/`false` for a flag).
    pub text: String,
    /// View-only field (secret or service).
    pub locked: bool,
}

/// The web edits the page (otherwise view-only).
pub fn editable(page: &str) -> bool {
    WEB_EDITABLE.contains(&page)
}

/// Secret field name: the value is neither shown nor written (`*_ref` is a reference, not a secret).
pub fn is_secret_key(key: &str) -> bool {
    const WORDS: [&str; 8] = [
        "token",
        "secret",
        "password",
        "passwd",
        "apikey",
        "api_key",
        "private_key",
        "authorization",
    ];
    let low = key.to_lowercase();
    !low.ends_with("_ref") && WORDS.iter().any(|w| low.contains(w))
}

/// Page fields from the core document; `rev` is not a field.
pub fn fields(page: &str, doc: &Value) -> Vec<Field> {
    let Some(obj) = doc.as_object() else {
        return Vec::new();
    };
    obj.iter()
        .filter(|(k, _)| k.as_str() != "rev")
        .map(|(k, v)| {
            let (kind, text) = match (page, k.as_str(), v) {
                ("memory", "source", Value::String(s)) => {
                    (Kind::Choice(vec!["local", "mcp_memory", "both"]), s.clone())
                }
                ("theme", "theme", Value::String(s)) => {
                    (Kind::Choice(vec!["auto", "dark", "light"]), s.clone())
                }
                (_, _, Value::Bool(b)) => (Kind::Bool, b.to_string()),
                (_, _, Value::Number(n)) => (Kind::Number, n.to_string()),
                (_, _, Value::String(s)) => (Kind::Text, s.clone()),
                (_, _, Value::Null) => (Kind::Text, String::new()),
                (_, _, other) => (
                    Kind::Json,
                    serde_json::to_string_pretty(other).unwrap_or_default(),
                ),
            };
            Field {
                locked: is_secret_key(k),
                key: k.clone(),
                kind,
                text,
            }
        })
        .collect()
}

/// Patch from edits `(key, text)`: only changed fields; text is parsed by the kind of the original value.
pub fn build_patch(doc: &Value, edits: &[(String, String)]) -> Result<Value, Vec<FieldErr>> {
    let mut patch = Map::new();
    let mut errs = Vec::new();
    let bad = |field: &str, message: &str| FieldErr {
        field: field.to_string(),
        message: message.to_string(),
    };
    for (key, text) in edits {
        if key == "rev" || is_secret_key(key) {
            continue;
        }
        let Some(old) = doc.get(key) else {
            continue;
        };
        let new = match old {
            Value::Bool(_) => match text.trim() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => {
                    errs.push(bad(key, "must be true or false"));
                    continue;
                }
            },
            Value::Number(n) => {
                let t = text.trim();
                let parsed = if n.is_f64() {
                    t.parse::<f64>().ok().and_then(serde_json::Number::from_f64)
                } else {
                    t.parse::<i64>().ok().map(serde_json::Number::from)
                };
                match parsed {
                    Some(p) => Value::Number(p),
                    None => {
                        errs.push(bad(key, "must be a number"));
                        continue;
                    }
                }
            }
            Value::String(_) => Value::String(text.clone()),
            Value::Null => {
                if text.is_empty() {
                    continue;
                }
                Value::String(text.clone())
            }
            _ => match serde_json::from_str::<Value>(text) {
                Ok(v) => v,
                Err(_) => {
                    errs.push(bad(key, "must be valid JSON"));
                    continue;
                }
            },
        };
        if &new != old {
            patch.insert(key.clone(), new);
        }
    }
    if errs.is_empty() {
        Ok(Value::Object(patch))
    } else {
        Err(errs)
    }
}

/// Page document `rev` (also the `ETag`).
pub fn doc_rev(doc: &Value) -> Option<i64> {
    doc.get("rev")?.as_i64()
}

/// Field errors from a `422` body (`error.details.fields`).
pub fn api_field_errors(body: &Value) -> Vec<FieldErr> {
    body.pointer("/error/details/fields")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(FieldErr {
                        field: f.get("field")?.as_str()?.to_string(),
                        message: f.get("message")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edits(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    #[test]
    fn editable_pages_are_the_safe_ones() {
        for p in WEB_EDITABLE {
            assert!(editable(p), "{p}");
        }
        for p in [
            "models",
            "connections",
            "permissions",
            "security",
            "tunnels",
            "modules",
            "about",
            "",
        ] {
            assert!(!editable(p), "{p}");
        }
    }

    #[test]
    fn secret_keys_by_name_but_not_refs() {
        for k in [
            "api_key",
            "Token",
            "client_secret",
            "password",
            "private_key",
        ] {
            assert!(is_secret_key(k), "{k}");
        }
        for k in ["api_key_ref", "secret_ref", "name", "source", "limit"] {
            assert!(!is_secret_key(k), "{k}");
        }
    }

    #[test]
    fn fields_follow_value_kinds_and_skip_rev() {
        let doc = json!({"rev": 3, "name": "John", "enabled": true, "limit": 5, "nick": null,
                         "hours": {"from": 9}, "source": "local", "api_key": "x"});
        let f = fields("memory", &doc);
        let by = |k: &str| f.iter().find(|x| x.key == k).cloned();
        assert!(by("rev").is_none());
        assert_eq!(by("name").unwrap().kind, Kind::Text);
        assert_eq!(by("name").unwrap().text, "John");
        assert_eq!(by("enabled").unwrap().kind, Kind::Bool);
        assert_eq!(by("enabled").unwrap().text, "true");
        assert_eq!(by("limit").unwrap().kind, Kind::Number);
        assert_eq!(by("nick").unwrap().text, "");
        assert_eq!(by("hours").unwrap().kind, Kind::Json);
        assert_eq!(
            by("hours").unwrap().text.replace(char::is_whitespace, ""),
            "{\"from\":9}"
        );
        assert_eq!(
            by("source").unwrap().kind,
            Kind::Choice(vec!["local", "mcp_memory", "both"])
        );
        assert!(by("api_key").unwrap().locked);
        assert!(!by("name").unwrap().locked);
        assert_eq!(f.len(), 7);
    }

    #[test]
    fn theme_choice() {
        let f = fields("theme", &json!({"rev": 1, "theme": "auto"}));
        assert_eq!(f[0].kind, Kind::Choice(vec!["auto", "dark", "light"]));
    }

    #[test]
    fn patch_has_only_changed_fields_with_right_types() {
        let doc = json!({"rev": 3, "name": "John", "enabled": true, "limit": 5, "ratio": 0.5,
                         "hours": {"from": 9}, "nick": null});
        let p = build_patch(
            &doc,
            &edits(&[
                ("name", "John"),
                ("enabled", "false"),
                ("limit", "7"),
                ("ratio", "0.25"),
                ("hours", "{\"from\": 10}"),
                ("nick", "Johnny"),
            ]),
        )
        .unwrap();
        assert_eq!(
            p,
            json!({"enabled": false, "limit": 7, "ratio": 0.25, "hours": {"from": 10}, "nick": "Johnny"})
        );
        assert_eq!(
            build_patch(&doc, &edits(&[("name", "John"), ("nick", "")])).unwrap(),
            json!({})
        );
    }

    #[test]
    fn patch_reports_bad_values_per_field() {
        let doc = json!({"limit": 5, "enabled": true, "hours": {}});
        let e = build_patch(
            &doc,
            &edits(&[("limit", "many"), ("enabled", "maybe"), ("hours", "{")]),
        )
        .unwrap_err();
        let names: Vec<&str> = e.iter().map(|x| x.field.as_str()).collect();
        assert_eq!(names, vec!["limit", "enabled", "hours"]);
        assert!(e.iter().all(|x| !x.message.is_empty()));
        assert!(e[0].message.contains("number"), "{e:?}");
        assert!(e[1].message.contains("true"), "{e:?}");
        assert!(e[2].message.contains("JSON"), "{e:?}");
    }

    #[test]
    fn patch_flag_both_directions_and_integer_stays_integer() {
        let doc = json!({"on": false, "n": 5, "f": 0.5});
        let p = build_patch(&doc, &edits(&[("on", "true"), ("n", "7"), ("f", "1")])).unwrap();
        assert_eq!(p, json!({"on": true, "n": 7, "f": 1.0}));
        assert!(p["n"].is_i64());
        // An integer field does not accept a fraction, a flag does not accept arbitrary JSON.
        let e = build_patch(&doc, &edits(&[("n", "7.5")])).unwrap_err();
        assert_eq!(e[0].field, "n");
        let e = build_patch(&doc, &edits(&[("on", "1")])).unwrap_err();
        assert_eq!(e[0].field, "on");
    }

    #[test]
    fn patch_never_carries_secrets_or_unknown_keys() {
        let doc = json!({"api_key": "x", "name": "a"});
        let p = build_patch(
            &doc,
            &edits(&[("api_key", "new"), ("ghost", "1"), ("name", "b")]),
        )
        .unwrap();
        assert_eq!(p, json!({"name": "b"}));
    }

    #[test]
    fn rev_and_api_errors() {
        assert_eq!(doc_rev(&json!({"rev": 12})), Some(12));
        assert_eq!(doc_rev(&json!({"rev": "x"})), None);
        assert_eq!(doc_rev(&json!({})), None);
        let e = api_field_errors(&json!({"error": {"code": "validation_failed", "details":
            {"fields": [{"field": "source", "message": "only local allowed"}, {"x": 1}]}}}));
        assert_eq!(
            e,
            vec![FieldErr {
                field: "source".into(),
                message: "only local allowed".into()
            }]
        );
        assert!(api_field_errors(&json!({"error": {"details": {}}})).is_empty());
    }
}
