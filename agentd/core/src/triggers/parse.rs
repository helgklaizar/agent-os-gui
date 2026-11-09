//! Validation of `POST /triggers` and `PATCH /triggers/{id}` bodies; errors are per field (`match.from`, `session_id`, …).

use serde_json::{Map, Value};

use crate::FieldError;

const MAX_SOURCE: usize = 16;
const MAX_MATCH_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerCreate {
    pub source: String,
    pub match_: Value,
    pub session_id: i64,
    pub task_id: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriggerEdit {
    pub source: Option<String>,
    pub match_: Option<Value>,
    pub session_id: Option<i64>,
    /// `Some(None)` — remove the action (`task_id: null`).
    pub task_id: Option<Option<i64>>,
    pub enabled: Option<bool>,
}

fn err(field: &str, msg: &str) -> FieldError {
    FieldError::new(field, msg)
}

fn source(v: &Value, errs: &mut Vec<FieldError>) -> Option<String> {
    match v.as_str() {
        Some(s)
            if (1..=MAX_SOURCE).contains(&s.len())
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
        {
            Some(s.to_string())
        }
        _ => {
            errs.push(err(
                "source",
                "channel: 1 to 16 characters of a-z, 0-9, \"_\" (tg, wa, mail)",
            ));
            None
        }
    }
}

fn match_doc(v: &Value, errs: &mut Vec<FieldError>) -> Option<Value> {
    let Some(obj) = v.as_object() else {
        errs.push(err("match", "an object {from?, contains?}"));
        return None;
    };
    let mut out = Map::new();
    for (k, val) in obj {
        let field = format!("match.{k}");
        if !matches!(k.as_str(), "from" | "contains") {
            errs.push(err(&field, "unknown field"));
            continue;
        }
        match val
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_MATCH_CHARS)
        {
            Some(s) => {
                out.insert(k.clone(), Value::String(s.to_string()));
            }
            None => errs.push(err(&field, "a non-empty string up to 200 characters")),
        }
    }
    Some(Value::Object(out))
}

fn session(v: &Value, errs: &mut Vec<FieldError>) -> Option<i64> {
    v.as_i64().filter(|n| *n > 0).or_else(|| {
        errs.push(err("session_id", "a positive integer"));
        None
    })
}

fn task(v: &Value, errs: &mut Vec<FieldError>) -> Option<Option<i64>> {
    if v.is_null() {
        return Some(None);
    }
    v.as_i64().filter(|n| *n > 0).map(Some).or_else(|| {
        errs.push(err("task_id", "a positive integer or null"));
        None
    })
}

fn enabled(v: &Value, errs: &mut Vec<FieldError>) -> Option<bool> {
    v.as_bool().or_else(|| {
        errs.push(err("enabled", "true or false"));
        None
    })
}

fn unknown(obj: &Map<String, Value>, errs: &mut Vec<FieldError>) {
    for k in obj
        .keys()
        .filter(|k| !["source", "match", "session_id", "task_id", "enabled"].contains(&k.as_str()))
    {
        errs.push(err(k, "unknown field"));
    }
}

pub fn parse_create(body: &Value) -> Result<TriggerCreate, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![err("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(obj, &mut errs);
    let src = source(obj.get("source").unwrap_or(&Value::Null), &mut errs);
    let m = match obj.get("match") {
        None => Some(Value::Object(Map::new())),
        Some(v) => match_doc(v, &mut errs),
    };
    let sid = session(obj.get("session_id").unwrap_or(&Value::Null), &mut errs);
    let en = match obj.get("enabled") {
        None => Some(true),
        Some(v) => enabled(v, &mut errs),
    };
    let task_id = obj
        .get("task_id")
        .and_then(|v| task(v, &mut errs))
        .flatten();
    match (src, m, sid, en, errs.is_empty()) {
        (Some(source), Some(match_), Some(session_id), Some(enabled), true) => Ok(TriggerCreate {
            source,
            match_,
            session_id,
            task_id,
            enabled,
        }),
        _ => Err(errs),
    }
}

pub fn parse_edit(body: &Value) -> Result<TriggerEdit, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![err("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(obj, &mut errs);
    let e = TriggerEdit {
        source: obj.get("source").and_then(|v| source(v, &mut errs)),
        match_: obj.get("match").and_then(|v| match_doc(v, &mut errs)),
        session_id: obj.get("session_id").and_then(|v| session(v, &mut errs)),
        task_id: obj.get("task_id").and_then(|v| task(v, &mut errs)),
        enabled: obj.get("enabled").and_then(|v| enabled(v, &mut errs)),
    };
    if errs.is_empty() { Ok(e) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(b: &Value) -> Vec<String> {
        parse_create(b)
            .unwrap_err()
            .into_iter()
            .map(|e| e.field)
            .collect()
    }

    #[test]
    fn create_defaults_and_trims_match() {
        let c = parse_create(&json!({"source": "tg", "match": {"from": " @a "}, "session_id": 3}))
            .unwrap();
        assert_eq!((c.match_, c.enabled), (json!({"from": "@a"}), true));
        assert_eq!(
            parse_create(&json!({"source": "wa", "session_id": 1}))
                .unwrap()
                .match_,
            json!({})
        );
    }

    #[test]
    fn bad_source_and_match_fields_are_named() {
        assert_eq!(names(&json!({"source": "TG", "session_id": 1})), ["source"]);
        assert_eq!(
            names(&json!({"source": "tg", "match": {"x": "y"}, "session_id": 1})),
            ["match.x"]
        );
        assert_eq!(
            names(&json!({"source": "tg", "match": {"from": ""}, "session_id": 0})),
            ["match.from", "session_id"]
        );
    }

    #[test]
    fn task_id_forms_in_create_and_edit() {
        let base = |t: Value| json!({"source": "tg", "session_id": 1, "task_id": t});
        assert_eq!(parse_create(&base(json!(7))).unwrap().task_id, Some(7));
        assert_eq!(parse_create(&base(Value::Null)).unwrap().task_id, None);
        assert_eq!(
            parse_create(&json!({"source": "tg", "session_id": 1}))
                .unwrap()
                .task_id,
            None
        );
        for bad in [json!(0), json!(-1), json!("7"), json!(1.5), json!(true)] {
            let e = parse_create(&base(bad.clone())).unwrap_err();
            assert_eq!(e.len(), 1, "{bad}");
            assert_eq!(e[0].field, "task_id", "{bad}");
        }
        // PATCH: no field — leave alone, null — remove, number — set.
        assert_eq!(parse_edit(&json!({})).unwrap().task_id, None);
        assert_eq!(
            parse_edit(&json!({"task_id": null})).unwrap().task_id,
            Some(None)
        );
        assert_eq!(
            parse_edit(&json!({"task_id": 3})).unwrap().task_id,
            Some(Some(3))
        );
        assert!(parse_edit(&json!({"task_id": 0})).is_err());
    }

    #[test]
    fn unknown_fields_are_named_in_create_and_edit() {
        assert_eq!(
            names(&json!({"source": "tg", "session_id": 1, "zzz": 1})),
            ["zzz"]
        );
        let e = parse_edit(&json!({"enabled": true, "extra": 2})).unwrap_err();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].field, "extra");
        assert_eq!(
            parse_edit(&json!({"enabled": false})).unwrap().enabled,
            Some(false)
        );
        assert!(
            !parse_create(&json!({"source": "tg", "session_id": 1, "enabled": false}))
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn source_alphabet_is_lowercase_digits_underscore() {
        for ok in ["tg", "tg2", "my_src", "mcp", "9", "_"] {
            assert!(
                parse_create(&json!({"source": ok, "session_id": 1})).is_ok(),
                "{ok}"
            );
        }
        for bad in ["TG", "tg-2", "tg 2", "tg.x", "é", "a".repeat(17).as_str()] {
            assert_eq!(
                names(&json!({"source": bad, "session_id": 1})),
                ["source"],
                "{bad}"
            );
        }
    }

    #[test]
    fn edit_is_partial() {
        let e = parse_edit(&json!({"enabled": false})).unwrap();
        assert_eq!((e.enabled, e.source), (Some(false), None));
        assert!(parse_edit(&json!({"enabled": 1})).is_err());
    }
}
