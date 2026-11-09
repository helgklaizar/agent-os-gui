//! Data of the Schedule and Search screens: parsing core replies (docs/reference/api.md sections 5, 6). Incomplete entries are skipped.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub spec: String,
    pub cron: Option<String>,
    pub state: String,
    pub next_run: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    pub started_at: i64,
    pub status: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: String,
    pub title: String,
    pub snippet: String,
    pub session_id: Option<i64>,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)?.as_str().map(str::to_string)
}

pub fn parse_tasks(data: &Value) -> Vec<TaskRow> {
    data.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(TaskRow {
                        id: t.get("id")?.as_i64()?,
                        kind: text(t, "kind").unwrap_or_default(),
                        title: text(t, "title")?,
                        spec: text(t, "spec").unwrap_or_default(),
                        cron: text(t, "cron"),
                        state: text(t, "state").unwrap_or_default(),
                        next_run: t.get("next_run").and_then(Value::as_i64),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_runs(data: &Value) -> Vec<RunRow> {
    data.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(RunRow {
                        id: r.get("id")?.as_i64()?,
                        started_at: r.get("started_at")?.as_i64()?,
                        status: text(r, "status").unwrap_or_default(),
                        output: text(r, "output").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reply of `GET /search`: `{items: [{kind, title, snippet, ref}]}`; the session is taken from `ref.session_id`.
pub fn parse_search(data: &Value) -> Vec<Hit> {
    data.get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    Some(Hit {
                        kind: text(h, "kind")?,
                        title: text(h, "title").unwrap_or_default(),
                        snippet: text(h, "snippet").unwrap_or_default(),
                        session_id: h
                            .get("ref")
                            .and_then(|r| r.get("session_id"))
                            .and_then(Value::as_i64),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Next task state for the Pause and Resume button; a finished task does not toggle.
pub fn toggled_state(state: &str) -> Option<&'static str> {
    match state {
        "active" => Some("paused"),
        "paused" => Some("active"),
        _ => None,
    }
}

/// Action log row (`GET /actions`, docs/reference/api.md section 5.4): the core stores masked arguments, they are not needed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRow {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub decision: String,
    pub result: String,
}

/// Log reply: `{items: [...], next_cursor?}` or an array; incomplete rows are skipped.
pub fn parse_actions(data: &Value) -> (Vec<ActionRow>, Option<String>) {
    let list = data
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| data.as_array());
    let rows = list
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(ActionRow {
                        id: r.get("id")?.as_i64()?,
                        ts: r.get("ts").and_then(Value::as_i64).unwrap_or(0),
                        actor: text(r, "actor").unwrap_or_default(),
                        action: text(r, "action")?,
                        target: text(r, "target").unwrap_or_default(),
                        decision: text(r, "decision").unwrap_or_default(),
                        result: text(r, "result").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (rows, text(data, "next_cursor"))
}

/// Body of `POST /sessions/{id}/tasks`: `sched` needs a five-field `cron`, `deleg` has none.
pub fn build_task_create(
    kind: &str,
    title: &str,
    spec: &str,
    cron: &str,
) -> Result<Value, Vec<(String, String)>> {
    let (title, spec, cron) = (title.trim(), spec.trim(), cron.trim());
    let mut errs = Vec::new();
    if !matches!(kind, "sched" | "deleg") {
        errs.push(("kind".to_string(), "sched or deleg".to_string()));
    }
    if title.is_empty() {
        errs.push(("title".to_string(), "title required".to_string()));
    }
    if spec.is_empty() {
        errs.push(("spec".to_string(), "description required".to_string()));
    }
    if kind == "sched" && !cron_shape_ok(cron) {
        errs.push((
            "cron".to_string(),
            "cron with five fields: minute hour day month weekday".to_string(),
        ));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut v = json!({"kind": kind, "title": title, "spec": spec});
    if kind == "sched" {
        v["cron"] = json!(cron);
    }
    Ok(v)
}

/// Body of `PATCH /tasks/{id}`: only changed `title`, `spec`, `cron`.
pub fn build_task_patch(
    task: &TaskRow,
    title: &str,
    spec: &str,
    cron: &str,
) -> Result<Value, Vec<(String, String)>> {
    let (title, spec, cron) = (title.trim(), spec.trim(), cron.trim());
    let mut errs = Vec::new();
    let mut patch = serde_json::Map::new();
    if title.is_empty() {
        errs.push(("title".to_string(), "title required".to_string()));
    } else if title != task.title {
        patch.insert("title".into(), json!(title));
    }
    if spec.is_empty() {
        errs.push(("spec".to_string(), "description required".to_string()));
    } else if spec != task.spec {
        patch.insert("spec".into(), json!(spec));
    }
    if task.cron.is_some() {
        if !cron_shape_ok(cron) {
            errs.push((
                "cron".to_string(),
                "cron with five fields: minute hour day month weekday".to_string(),
            ));
        } else if Some(cron) != task.cron.as_deref() {
            patch.insert("cron".into(), json!(cron));
        }
    }
    if errs.is_empty() {
        Ok(Value::Object(patch))
    } else {
        Err(errs)
    }
}

/// Cron of five fields (minute hour day month weekday); value parsing stays with the core.
pub fn cron_shape_ok(s: &str) -> bool {
    s.split_whitespace().count() == 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(cron: Option<&str>) -> TaskRow {
        TaskRow {
            id: 1,
            kind: if cron.is_some() { "sched" } else { "deleg" }.into(),
            title: "morning".into(),
            spec: "summary".into(),
            cron: cron.map(str::to_string),
            state: "active".into(),
            next_run: None,
        }
    }

    #[test]
    fn create_sched_trims_and_needs_cron() {
        let v = build_task_create("sched", " morning ", " summary ", " 0 8 * * * ").unwrap();
        assert_eq!(
            v,
            json!({"kind": "sched", "title": "morning", "spec": "summary", "cron": "0 8 * * *"})
        );
        let e = build_task_create("sched", "t", "s", "").unwrap_err();
        assert_eq!(e[0].0, "cron");
        let e = build_task_create("sched", "t", "s", "0 8 * *").unwrap_err();
        assert_eq!(e[0].0, "cron");
    }

    #[test]
    fn create_deleg_has_no_cron_and_each_field_reported() {
        let v = build_task_create("deleg", "t", "s", "").unwrap();
        assert_eq!(v, json!({"kind": "deleg", "title": "t", "spec": "s"}));
        let e = build_task_create("deleg", "", "", "").unwrap_err();
        assert_eq!(
            e.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            vec!["title", "spec"]
        );
        let e = build_task_create("x", "t", "s", "").unwrap_err();
        assert_eq!(e[0].0, "kind");
    }

    #[test]
    fn patch_changes_only_what_changed() {
        let t = row(Some("0 8 * * *"));
        assert_eq!(
            build_task_patch(&t, "morning", "summary", "0 8 * * *").unwrap(),
            json!({})
        );
        assert_eq!(
            build_task_patch(&t, "evening", "summary", "0 20 * * *").unwrap(),
            json!({"title": "evening", "cron": "0 20 * * *"})
        );
        let e = build_task_patch(&t, "", "summary", "0 8 * * *").unwrap_err();
        assert_eq!(e[0].0, "title");
        let e = build_task_patch(&t, "morning", "summary", "bogus").unwrap_err();
        assert_eq!(e[0].0, "cron");
        // `deleg` has no schedule and never gets one.
        let d = row(None);
        assert_eq!(
            build_task_patch(&d, "morning", "summary", "").unwrap(),
            json!({})
        );
    }

    #[test]
    fn cron_shape() {
        assert!(cron_shape_ok("0 8 * * *"));
        assert!(cron_shape_ok("*/5  9-18 * * 1-5"));
        assert!(!cron_shape_ok("0 8 * *"));
        assert!(!cron_shape_ok("0 8 * * * *"));
        assert!(!cron_shape_ok(""));
    }

    #[test]
    fn actions_parse_items_and_cursor() {
        let (a, c) = parse_actions(&json!({"items": [
            {"id": 2, "ts": 9, "actor": "agent", "action": "shell", "target": "ls", "decision": "allow", "result": "ok"},
            {"id": 1, "ts": 8, "actor": "user", "action": "send"},
            {"ts": 7}
        ], "next_cursor": "abc"}));
        assert_eq!(a.len(), 2);
        assert_eq!(
            (a[0].action.as_str(), a[0].target.as_str()),
            ("shell", "ls")
        );
        assert_eq!(a[1].decision, "");
        assert_eq!(c.as_deref(), Some("abc"));
        let (a, c) = parse_actions(&json!([{"id": 5, "ts": 1, "actor": "cron", "action": "run"}]));
        assert_eq!((a.len(), c), (1, None));
        assert!(parse_actions(&json!("x")).0.is_empty());
    }

    #[test]
    fn tasks_skip_incomplete() {
        let t = parse_tasks(&json!([
            {"id": 1, "kind": "sched", "title": "<b>a</b>", "spec": "what to do", "cron": "0 9 * * *", "state": "active", "next_run": 10},
            {"id": 2, "title": "no kind"},
            {"id": 3},
            {"title": "no id"}
        ]));
        assert_eq!(t.len(), 2);
        assert_eq!(
            (t[0].id, t[0].title.as_str(), t[0].next_run),
            (1, "<b>a</b>", Some(10))
        );
        assert_eq!(t[0].spec, "what to do");
        assert_eq!(t[1].cron, None);
        assert!(parse_tasks(&json!({"x": 1})).is_empty());
    }

    #[test]
    fn runs_need_id_and_start() {
        let r = parse_runs(
            &json!([{"id": 1, "started_at": 5, "status": "ok", "output": "done"}, {"id": 2}]),
        );
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].status.as_str(), r[0].output.as_str()), ("ok", "done"));
    }

    #[test]
    fn search_hits_with_and_without_session() {
        let h = parse_search(&json!({"items": [
            {"kind": "message", "title": "t", "snippet": "s", "ref": {"session_id": 7, "message_id": 2}},
            {"kind": "memory", "title": "m", "snippet": "x", "ref": {"id": 1}},
            {"title": "no kind"}
        ]}));
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].session_id, Some(7));
        assert_eq!(h[1].session_id, None);
        assert!(parse_search(&json!([])).is_empty());
    }

    #[test]
    fn toggle_pause_resume_only() {
        assert_eq!(toggled_state("active"), Some("paused"));
        assert_eq!(toggled_state("paused"), Some("active"));
        assert_eq!(toggled_state("done"), None);
        assert_eq!(toggled_state(""), None);
    }
}
