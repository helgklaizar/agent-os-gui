//! Bar indicators (`bar.indicators`, docs/reference/api.md section 4.4): states `ok|warn|bad|nodata|unknown`, snapshot and delta.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ind {
    pub id: String,
    pub state: String,
    pub text: String,
}

/// One state; without `id` it is `None`.
pub fn parse_one(v: &Value) -> Option<Ind> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Ind {
        id: text("id")?,
        state: text("state").unwrap_or_else(|| "unknown".to_string()),
        text: text("text").unwrap_or_default(),
    })
}

/// Snapshot `{items: [...]}`.
pub fn parse_snapshot(data: &Value) -> Vec<Ind> {
    data.get("items")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_one).collect())
        .unwrap_or_default()
}

/// Delta: one state replaces or adds the entry with the same `id`.
pub fn apply_delta(list: &mut Vec<Ind>, data: &Value) {
    if let Some(n) = parse_one(data) {
        match list.iter_mut().find(|i| i.id == n.id) {
            Some(old) => *old = n,
            None => list.push(n),
        }
    }
}

/// Color class; an unknown state counts as "no data".
pub fn state_class(state: &str) -> &'static str {
    match state {
        "ok" => "l-ok",
        "warn" => "l-warn",
        "bad" => "l-bad",
        _ => "l-nodata",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_and_delta() {
        let mut l = parse_snapshot(&json!({"items": [
            {"id": "cpu", "state": "ok", "text": "14%"},
            {"id": "whapi", "state": "bad"},
            {"state": "ok"}
        ]}));
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].text, "");
        apply_delta(&mut l, &json!({"id": "whapi", "state": "ok", "text": "QR"}));
        assert_eq!((l[1].state.as_str(), l[1].text.as_str()), ("ok", "QR"));
        apply_delta(&mut l, &json!({"id": "backup", "state": "warn"}));
        assert_eq!(l.len(), 3);
        apply_delta(&mut l, &json!({"x": 1}));
        assert_eq!(l.len(), 3);
        assert!(parse_one(&json!({"id": "a"})).is_some());
        assert_eq!(parse_one(&json!({"id": "a"})).unwrap().state, "unknown");
    }

    #[test]
    fn classes() {
        assert_eq!(state_class("ok"), "l-ok");
        assert_eq!(state_class("warn"), "l-warn");
        assert_eq!(state_class("bad"), "l-bad");
        assert_eq!(state_class("nodata"), "l-nodata");
        assert_eq!(state_class("unknown"), "l-nodata");
        assert_eq!(state_class("something"), "l-nodata");
    }
}
