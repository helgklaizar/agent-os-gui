//! Parsing of feed requests: the `GET /events` filter and the `POST /events/read` body. Errors are per field (`422`).

use std::collections::HashMap;

use serde_json::Value;

use super::{
    DEFAULT_LIMIT, EventFilter, KINDS, MAX_LIMIT, MAX_READ_IDS, MAX_SUB_CHARS, ReadSelector,
};
use crate::FieldError;

fn bad(field: &str, msg: &str) -> FieldError {
    FieldError::new(field, msg)
}

fn kind(v: &str, errs: &mut Vec<FieldError>) -> Option<String> {
    if KINDS.contains(&v) {
        Some(v.to_string())
    } else {
        errs.push(bad("kind", "allowed: ai, task or msg"));
        None
    }
}

fn sub(v: &str, errs: &mut Vec<FieldError>) -> Option<String> {
    if !v.is_empty() && v.chars().count() <= MAX_SUB_CHARS {
        Some(v.to_string())
    } else {
        errs.push(bad("sub", "1 to 32 characters"));
        None
    }
}

fn positive(field: &str, v: &str, errs: &mut Vec<FieldError>) -> Option<i64> {
    let n = v.parse::<i64>().ok().filter(|n| *n > 0);
    if n.is_none() {
        errs.push(bad(field, "a positive integer"));
    }
    n
}

/// Query string parameters; values without percent-encoding (numbers, `true`/`false`, subtype names).
pub fn parse_filter(q: &HashMap<String, String>) -> Result<EventFilter, Vec<FieldError>> {
    let mut errs = Vec::new();
    let mut f = EventFilter::default();
    for k in q.keys() {
        if !["kind", "sub", "unread", "session_id", "before", "limit"].contains(&k.as_str()) {
            errs.push(bad(k, "unknown parameter"));
        }
    }
    f.kind = q.get("kind").and_then(|v| kind(v, &mut errs));
    f.sub = q.get("sub").and_then(|v| sub(v, &mut errs));
    f.session_id = q
        .get("session_id")
        .and_then(|v| positive("session_id", v, &mut errs));
    f.before = q
        .get("before")
        .and_then(|v| positive("before", v, &mut errs));
    match q.get("unread").map(String::as_str) {
        None => {}
        Some("true") => f.unread = Some(true),
        Some("false") => f.unread = Some(false),
        Some(_) => errs.push(bad("unread", "true or false")),
    }
    if let Some(v) = q.get("limit") {
        match v.parse::<i64>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => f.limit = n,
            _ => errs.push(bad("limit", "1 to 200")),
        }
    } else {
        f.limit = DEFAULT_LIMIT;
    }
    if errs.is_empty() { Ok(f) } else { Err(errs) }
}

/// `POST /events/read` body: `ids`, `kind`, `sub`, `session_id`; `null`, `{}` and empty — all unread.
pub fn parse_read(body: &Value) -> Result<ReadSelector, Vec<FieldError>> {
    let mut sel = ReadSelector::default();
    if body.is_null() {
        return Ok(sel);
    }
    let Some(o) = body.as_object() else {
        return Err(vec![bad("", "an object is expected")]);
    };
    let mut errs: Vec<FieldError> = o
        .keys()
        .filter(|k| !["ids", "kind", "sub", "session_id"].contains(&k.as_str()))
        .map(|k| bad(k, "unknown field"))
        .collect();
    if let Some(v) = o.get("kind").filter(|v| !v.is_null()) {
        sel.kind = v.as_str().and_then(|s| kind(s, &mut errs));
        if v.as_str().is_none() {
            errs.push(bad("kind", "a string is expected"));
        }
    }
    if let Some(v) = o.get("sub").filter(|v| !v.is_null()) {
        match v.as_str() {
            Some(s) => sel.sub = sub(s, &mut errs),
            None => errs.push(bad("sub", "a string is expected")),
        }
    }
    if let Some(v) = o.get("session_id").filter(|v| !v.is_null()) {
        match v.as_i64().filter(|n| *n > 0) {
            Some(n) => sel.session_id = Some(n),
            None => errs.push(bad("session_id", "a positive integer")),
        }
    }
    if let Some(v) = o.get("ids").filter(|v| !v.is_null()) {
        let ids: Option<Vec<i64>> = v
            .as_array()
            .filter(|a| a.len() <= MAX_READ_IDS)
            .and_then(|a| a.iter().map(Value::as_i64).collect());
        match ids {
            Some(ids) => sel.ids = Some(ids),
            None => errs.push(bad("ids", "an array of integers, at most 1000")),
        }
    }
    if errs.is_empty() { Ok(sel) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn filter_defaults_and_bounds() {
        let f = parse_filter(&q(&[])).unwrap();
        assert_eq!((f.limit, f.kind, f.unread), (DEFAULT_LIMIT, None, None));
        assert!(parse_filter(&q(&[("limit", "201")])).is_err());
        assert!(parse_filter(&q(&[("limit", "200")])).is_ok());
        assert!(parse_filter(&q(&[("kind", "AI")])).is_err());
        assert!(parse_filter(&q(&[("zzz", "1")])).is_err());
        assert!(parse_filter(&q(&[("session_id", "0")])).is_err());
    }

    #[test]
    fn read_body_forms() {
        assert_eq!(parse_read(&Value::Null).unwrap(), ReadSelector::default());
        assert_eq!(parse_read(&json!({})).unwrap(), ReadSelector::default());
        let s = parse_read(&json!({"ids": [1, 2], "kind": "msg"})).unwrap();
        assert_eq!(s.ids, Some(vec![1, 2]));
        assert!(parse_read(&json!({"ids": [1, "x"]})).is_err());
        assert!(parse_read(&json!({"ids": vec![1; 1001]})).is_err());
        assert!(parse_read(&json!([1])).is_err());
        assert!(parse_read(&json!({"kind": 5})).is_err());
    }
}
