//! Parsing of `events` frames and of the subtype registry response.

use controllers::notif_types::{parse_event, parse_read, parse_snapshot, parse_subtypes};
use serde_json::json;

fn event(id: i64) -> serde_json::Value {
    json!({"id": id, "kind": "msg", "sub": "wa", "level": "info", "text": "x", "session_id": null,
           "tab": "chat", "source": "agent-gw", "unread": true, "ts": 1})
}

#[test]
fn parse_event_reads_fields_and_ignores_unknown() {
    let mut v = event(5);
    v["extra"] = json!("new core field");
    let e = parse_event(&v).expect("event");
    assert_eq!((e.id, e.kind.as_str(), e.sub.as_str()), (5, "msg", "wa"));
    assert_eq!(e.session_id, None);
    assert!(e.unread);
    assert_eq!(e.source.as_deref(), Some("agent-gw"));
}

#[test]
fn parse_event_rejects_unknown_kind_and_garbage() {
    let mut bad = event(1);
    bad["kind"] = json!("bogus");
    assert!(parse_event(&bad).is_none(), "kind outside ai/task/msg");
    assert!(parse_event(&json!("string")).is_none());
    assert!(parse_event(&json!({"kind": "ai"})).is_none(), "without id");
}

#[test]
fn parse_snapshot_reads_items_and_counts_and_skips_bad_items() {
    let mut bad = event(2);
    bad["kind"] = json!("bogus");
    let data = json!({"items": [event(3), bad, event(1)],
                      "counts": {"total": 2, "by_kind": {"msg": 2}, "by_sub": {"msg.wa": 2}}});
    let (items, counts) = parse_snapshot(&data).expect("snapshot");
    assert_eq!(items.iter().map(|e| e.id).collect::<Vec<_>>(), vec![3, 1]);
    assert_eq!(counts.total, 2);
    assert_eq!(counts.by_kind.get("msg"), Some(&2));
    assert_eq!(counts.by_sub.get("msg.wa"), Some(&2));
    assert!(parse_snapshot(&json!({"items": "none"})).is_none());
    assert!(parse_snapshot(&json!({})).is_none());
}

#[test]
fn parse_read_and_subtypes() {
    assert_eq!(parse_read(&json!({"read": [3, 4]})), Some(vec![3, 4]));
    assert_eq!(parse_read(&json!({"read": ["x"]})), None);
    assert_eq!(parse_read(&json!({"other": 1})), None);
    let list = parse_subtypes(&json!([
        {"kind": "ai", "sub": "agent", "label": "Agent", "icon": "i", "enabled": true},
        {"kind": "msg", "sub": "x", "label": "X"},
        {"nope": 1}
    ]));
    assert_eq!(list.len(), 2, "a malformed entry is skipped");
    assert!(list[1].enabled, "enabled defaults to true");
    assert_eq!(list[1].icon, None);
    assert!(parse_subtypes(&json!({"not": "array"})).is_empty());
}
