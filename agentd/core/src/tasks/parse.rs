//! Validation of the `POST /sessions/{id}/tasks` and `PATCH /tasks/{id}` bodies and the `GET /tasks` query; errors are per field.

use std::collections::HashMap;

use serde_json::Value;

use super::{DEFAULT_TZ, MAX_SPEC_CHARS, MAX_TITLE_CHARS, TaskFilter, cron_check};
use crate::{CronError, FieldError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCreate {
    pub kind: String,
    pub title: String,
    pub spec: String,
    pub cron: Option<String>,
    pub tz: String,
    pub persona_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskEdit {
    pub title: Option<String>,
    pub spec: Option<String>,
    pub cron: Option<Option<String>>,
    pub tz: Option<String>,
    pub state: Option<String>,
    pub persona_id: Option<Option<i64>>,
}

fn err(field: &str, msg: &str) -> FieldError {
    FieldError::new(field, msg)
}

fn text(v: &Value, field: &str, max: usize, errs: &mut Vec<FieldError>) -> Option<String> {
    match v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if s.chars().count() <= max => Some(s.to_string()),
        _ => {
            errs.push(err(field, &format!("string of 1 to {max} characters")));
            None
        }
    }
}

fn unknown(obj: &serde_json::Map<String, Value>, allowed: &[&str], errs: &mut Vec<FieldError>) {
    for k in obj.keys().filter(|k| !allowed.contains(&k.as_str())) {
        errs.push(err(k, "unknown field"));
    }
}

fn cron_errors(cron: &str, tz: &str, errs: &mut Vec<FieldError>) {
    match cron_check(cron, tz) {
        Ok(()) => {}
        Err(CronError::Zone) => errs.push(err(
            "tz",
            "zone from the IANA database, e.g. Asia/Jerusalem",
        )),
        Err(CronError::Pattern) => errs.push(err(
            "cron",
            "five-field cron: minute hour day month weekday",
        )),
    }
}

fn persona(v: &Value, errs: &mut Vec<FieldError>) -> Option<i64> {
    match v {
        Value::Null => None,
        v => v.as_i64().filter(|n| *n > 0).or_else(|| {
            errs.push(err("persona_id", "positive integer or null"));
            None
        }),
    }
}

/// Body for creating a session task.
pub fn parse_create(body: &Value) -> Result<TaskCreate, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![err("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(
        obj,
        &["kind", "title", "spec", "cron", "tz", "persona_id"],
        &mut errs,
    );
    let kind = match obj.get("kind").and_then(Value::as_str) {
        Some(k @ ("sched" | "deleg")) => k.to_string(),
        _ => {
            errs.push(err("kind", "sched or deleg"));
            String::new()
        }
    };
    let title = text(
        obj.get("title").unwrap_or(&Value::Null),
        "title",
        MAX_TITLE_CHARS,
        &mut errs,
    );
    let spec = text(
        obj.get("spec").unwrap_or(&Value::Null),
        "spec",
        MAX_SPEC_CHARS,
        &mut errs,
    );
    let tz = match obj.get("tz") {
        None | Some(Value::Null) => DEFAULT_TZ.to_string(),
        Some(v) => v.as_str().unwrap_or_default().to_string(),
    };
    let cron = match obj.get("cron") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().unwrap_or_default().trim().to_string()),
    };
    match (kind.as_str(), &cron) {
        ("sched", None) => errs.push(err("cron", "sched needs a schedule")),
        ("deleg", Some(_)) => errs.push(err("cron", "deleg has no schedule")),
        (_, Some(c)) => cron_errors(c, &tz, &mut errs),
        _ => {}
    }
    if cron.is_none() && tz.parse::<chrono_tz::Tz>().is_err() {
        errs.push(err(
            "tz",
            "zone from the IANA database, e.g. Asia/Jerusalem",
        ));
    }
    let persona_id = persona(obj.get("persona_id").unwrap_or(&Value::Null), &mut errs);
    match (errs.is_empty(), title, spec) {
        (true, Some(title), Some(spec)) => Ok(TaskCreate {
            kind,
            title,
            spec,
            cron,
            tz,
            persona_id,
        }),
        _ => Err(errs),
    }
}

/// Edit body: any subset of `title`, `spec`, `cron`, `tz`, `state`, `persona_id`.
pub fn parse_edit(body: &Value) -> Result<TaskEdit, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![err("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(
        obj,
        &["title", "spec", "cron", "tz", "state", "persona_id"],
        &mut errs,
    );
    let mut e = TaskEdit::default();
    if let Some(v) = obj.get("title") {
        e.title = text(v, "title", MAX_TITLE_CHARS, &mut errs);
    }
    if let Some(v) = obj.get("spec") {
        e.spec = text(v, "spec", MAX_SPEC_CHARS, &mut errs);
    }
    if let Some(v) = obj.get("cron") {
        e.cron = Some(v.as_str().map(|s| s.trim().to_string()));
        if !v.is_null() && v.as_str().is_none() {
            errs.push(err("cron", "string or null"));
        }
    }
    if let Some(v) = obj.get("tz") {
        match v.as_str() {
            Some(s) => e.tz = Some(s.to_string()),
            None => errs.push(err("tz", "string")),
        }
    }
    if let Some(v) = obj.get("state") {
        match v.as_str() {
            Some(s @ ("active" | "paused" | "done")) => e.state = Some(s.to_string()),
            _ => errs.push(err("state", "active, paused or done")),
        }
    }
    if let Some(v) = obj.get("persona_id") {
        e.persona_id = Some(persona(v, &mut errs));
    }
    if errs.is_empty() { Ok(e) } else { Err(errs) }
}

/// `GET /tasks` filter: `kind`, `state`, `session_id`.
pub fn parse_filter(q: &HashMap<String, String>) -> Result<TaskFilter, Vec<FieldError>> {
    let mut errs = Vec::new();
    let mut f = TaskFilter::default();
    if let Some(k) = q.get("kind") {
        if matches!(k.as_str(), "sched" | "deleg") {
            f.kind = Some(k.clone());
        } else {
            errs.push(err("kind", "sched or deleg"));
        }
    }
    if let Some(s) = q.get("state") {
        if matches!(s.as_str(), "active" | "paused" | "done") {
            f.state = Some(s.clone());
        } else {
            errs.push(err("state", "active, paused or done"));
        }
    }
    if let Some(s) = q.get("session_id") {
        match s.parse::<i64>() {
            Ok(n) if n > 0 => f.session_id = Some(n),
            _ => errs.push(err("session_id", "positive integer")),
        }
    }
    if errs.is_empty() { Ok(f) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(r: Result<TaskCreate, Vec<FieldError>>) -> Vec<String> {
        r.unwrap_err().into_iter().map(|f| f.field).collect()
    }

    #[test]
    fn create_defaults_tz_and_trims() {
        let c = parse_create(
            &json!({"kind": "sched", "title": " morning ", "spec": "x", "cron": "0 8 * * *"}),
        )
        .unwrap();
        assert_eq!((c.title.as_str(), c.tz.as_str()), ("morning", DEFAULT_TZ));
        let d = parse_create(&json!({"kind": "deleg", "title": "t", "spec": "x"})).unwrap();
        assert!(d.cron.is_none());
    }

    #[test]
    fn create_reports_each_field() {
        let f = fields(parse_create(
            &json!({"kind": "sched", "title": "", "spec": "", "cron": "0 8 * *", "z": 1}),
        ));
        for want in ["title", "spec", "cron", "z"] {
            assert!(f.contains(&want.to_string()), "{want}: {f:?}");
        }
        assert_eq!(fields(parse_create(&json!([]))), ["body"]);
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(
            fields(parse_create(
                &json!({"kind": "deleg", "title": long, "spec": "x"})
            ))
            .contains(&"title".to_string())
        );
    }

    #[test]
    fn edit_accepts_subset_and_null_cron() {
        let e = parse_edit(&json!({"state": "paused", "cron": null})).unwrap();
        assert_eq!((e.state.as_deref(), e.cron), (Some("paused"), Some(None)));
        assert!(parse_edit(&json!({"state": "gone"})).is_err());
        assert!(
            parse_edit(&json!({"kind": "deleg"})).is_err(),
            "kind does not change"
        );
        assert!(parse_edit(&json!({"persona_id": 0})).is_err());
    }

    #[test]
    fn filter_validates_values() {
        let q = |k: &str, v: &str| HashMap::from([(k.to_string(), v.to_string())]);
        assert_eq!(
            parse_filter(&q("state", "active"))
                .unwrap()
                .state
                .as_deref(),
            Some("active")
        );
        assert!(parse_filter(&q("kind", "x")).is_err());
        assert!(parse_filter(&q("session_id", "-1")).is_err());
    }
}
