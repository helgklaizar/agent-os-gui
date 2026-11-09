//! Notifications: a core event (`Event`, docs/reference/api.md section 6) to the extent the feed needs. Text is output only as text nodes.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub kind: String,
    pub level: String,
    pub text: String,
    pub session_id: Option<i64>,
    pub unread: bool,
    pub ts: i64,
}

/// Event by shape; incomplete means `None` (the client skips such frames, docs/reference/api.md section 2).
pub fn parse_note(v: &Value) -> Option<Note> {
    let unread = match v.get("unread") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    };
    Some(Note {
        id: v.get("id")?.as_i64()?,
        kind: v
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("msg")
            .to_string(),
        level: v
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or("info")
            .to_string(),
        text: v.get("text")?.as_str()?.to_string(),
        session_id: v.get("session_id").and_then(Value::as_i64),
        unread,
        ts: v.get("ts").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// Feed from `snapshot` (`items`) or a `GET /events` reply; newest first.
pub fn parse_list(data: &Value) -> Vec<Note> {
    let mut v: Vec<Note> = data
        .get("items")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_note).collect())
        .unwrap_or_default();
    v.sort_by_key(|n| std::cmp::Reverse(n.id));
    v
}

/// New event to the front, at most `LIMIT` entries, a repeated `id` is replaced.
pub fn push(list: &mut Vec<Note>, n: Note) {
    list.retain(|x| x.id != n.id);
    list.insert(0, n);
    list.truncate(LIMIT);
}

/// Delta `{read: [ids]}`: marked read by another client.
pub fn mark_read(list: &mut [Note], ids: &[i64]) {
    for n in list.iter_mut().filter(|n| ids.contains(&n.id)) {
        n.unread = false;
    }
}

pub const LIMIT: usize = 100;

pub fn unread_count(list: &[Note]) -> usize {
    list.iter().filter(|n| n.unread).count()
}

/// Column filter: kind (`ai`, `task`, `msg`) and unread only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub kind: Option<String>,
    pub only_unread: bool,
}

/// Entries that pass the filter, order preserved.
pub fn apply(list: &[Note], f: &Filter) -> Vec<Note> {
    list.iter()
        .filter(|n| f.kind.as_deref().is_none_or(|k| n.kind == k))
        .filter(|n| !f.only_unread || n.unread)
        .cloned()
        .collect()
}

/// Event subtype as core data (`GET /events/subtypes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtype {
    pub kind: String,
    pub sub: String,
    pub label: String,
}

/// Enabled subtypes; an entry without `kind` or `sub` is skipped, without `label` the `sub` serves as the label.
pub fn parse_subtypes(data: &Value) -> Vec<Subtype> {
    data.as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s.get("enabled").and_then(Value::as_bool).unwrap_or(true))
                .filter_map(|s| {
                    let kind = s.get("kind")?.as_str()?.to_string();
                    let sub = s.get("sub")?.as_str()?.to_string();
                    let label = s
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|l| !l.is_empty())
                        .unwrap_or(&sub)
                        .to_string();
                    Some(Subtype { kind, sub, label })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Event kind icon (a name from `icons`).
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "ai" => "bot",
        "task" => "task",
        "msg" => "msg",
        _ => "bell",
    }
}

/// Card stripe color: an error overrides the kind.
pub fn stripe(kind: &str, level: &str) -> &'static str {
    if level == "error" {
        return "stripe-bad";
    }
    match kind {
        "task" => "stripe-task",
        "msg" => "stripe-msg",
        _ => "stripe-ai",
    }
}

/// System browser notification: only with consent, only for an unread event and only when the tab is hidden.
pub fn should_popup(n: &Note, opted_in: bool, page_hidden: bool) -> bool {
    opted_in && page_hidden && n.unread
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(id: i64, kind: &str, unread: bool) -> Note {
        parse_note(&json!({"id": id, "kind": kind, "text": "t", "unread": unread})).unwrap()
    }

    #[test]
    fn kind_icons_and_stripes() {
        assert_eq!(kind_icon("ai"), "bot");
        assert_eq!(kind_icon("task"), "task");
        assert_eq!(kind_icon("msg"), "msg");
        assert_eq!(kind_icon("something"), "bell");
        assert!(crate::icons::ALL.contains(&kind_icon("ai")));
        assert!(crate::icons::ALL.contains(&kind_icon("something")));
        assert_eq!(stripe("task", "info"), "stripe-task");
        assert_eq!(stripe("msg", "info"), "stripe-msg");
        assert_eq!(stripe("ai", "info"), "stripe-ai");
        assert_eq!(stripe("msg", "error"), "stripe-bad");
        assert_eq!(stripe("other", "info"), "stripe-ai");
    }

    #[test]
    fn filter_by_kind_and_unread() {
        let l = vec![n(3, "ai", true), n(2, "task", false), n(1, "ai", false)];
        let ids = |v: Vec<Note>| v.iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(ids(apply(&l, &Filter::default())), vec![3, 2, 1]);
        let ai = Filter {
            kind: Some("ai".into()),
            only_unread: false,
        };
        assert_eq!(ids(apply(&l, &ai)), vec![3, 1]);
        let un = Filter {
            kind: None,
            only_unread: true,
        };
        assert_eq!(ids(apply(&l, &un)), vec![3]);
        let both = Filter {
            kind: Some("task".into()),
            only_unread: true,
        };
        assert!(apply(&l, &both).is_empty());
    }

    #[test]
    fn subtypes_are_data_from_core() {
        let s = parse_subtypes(&json!([
            {"kind": "ai", "sub": "agent", "label": "Agent", "icon": "x", "enabled": true},
            {"kind": "task", "sub": "sched", "enabled": true},
            {"kind": "msg", "sub": "wa", "label": "WhatsApp", "enabled": false},
            {"sub": "no kind"}
        ]));
        assert_eq!(
            s,
            vec![
                Subtype {
                    kind: "ai".into(),
                    sub: "agent".into(),
                    label: "Agent".into()
                },
                Subtype {
                    kind: "task".into(),
                    sub: "sched".into(),
                    label: "sched".into()
                },
            ]
        );
        assert!(parse_subtypes(&json!({"x": 1})).is_empty());
    }

    #[test]
    fn popup_needs_consent_unread_and_hidden_page() {
        let u = n(1, "ai", true);
        let r = n(2, "ai", false);
        assert!(should_popup(&u, true, true));
        assert!(!should_popup(&u, false, true));
        assert!(!should_popup(&u, true, false));
        assert!(!should_popup(&r, true, true));
    }

    #[test]
    fn parses_event_and_ignores_malformed() {
        let n = parse_note(&json!({"id": 3, "kind": "ai", "level": "warn", "text": "<b>x</b>", "session_id": 7, "unread": true, "ts": 5})).unwrap();
        assert_eq!((n.id, n.session_id, n.unread), (3, Some(7), true));
        assert_eq!(n.text, "<b>x</b>");
        assert!(parse_note(&json!({"id": 1})).is_none());
        assert!(parse_note(&json!({"text": "x"})).is_none());
        let n = parse_note(&json!({"id": 2, "text": "t", "unread": 1})).unwrap();
        assert!(n.unread);
    }

    #[test]
    fn list_is_newest_first_and_bounded() {
        let l = parse_list(
            &json!({"items": [{"id": 1, "text": "a"}, {"id": 5, "text": "b"}, {"x": 1}]}),
        );
        assert_eq!(l.iter().map(|n| n.id).collect::<Vec<_>>(), vec![5, 1]);
        let mut l = Vec::new();
        for i in 0..(LIMIT as i64 + 20) {
            push(&mut l, parse_note(&json!({"id": i, "text": "t"})).unwrap());
        }
        assert_eq!(l.len(), LIMIT);
        assert_eq!(l[0].id, LIMIT as i64 + 19);
        push(
            &mut l,
            parse_note(&json!({"id": LIMIT as i64 + 19, "text": "again"})).unwrap(),
        );
        assert_eq!(l.len(), LIMIT);
        assert_eq!(l[0].text, "again");
    }

    #[test]
    fn read_marks_and_counts() {
        let mut l = parse_list(
            &json!({"items": [{"id": 1, "text": "a", "unread": true}, {"id": 2, "text": "b", "unread": true}]}),
        );
        assert_eq!(unread_count(&l), 2);
        mark_read(&mut l, &[2]);
        assert_eq!(unread_count(&l), 1);
        assert!(l.iter().find(|n| n.id == 1).unwrap().unread);
    }
}
