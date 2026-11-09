//! `sessions` frames from tree differences before and after a mutation: the client also gets indirect changes.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::FrameType;
use crate::state::Shared;
use crate::tree::Tree;

fn changed_only_unread(a: &Value, b: &Value) -> bool {
    let strip = |v: &Value| {
        let mut m = v.as_object().cloned().unwrap_or_default();
        m.remove("unread");
        m
    };
    strip(a) == strip(b)
}

fn sorted(m: &HashMap<i64, Value>) -> Vec<(&i64, &Value)> {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by_key(|(id, _)| **id);
    v
}

/// Publishes the differences between the tree before and after and remembers the new topic state.
pub fn publish_diff(shared: &Shared, before: &Tree, after: &Tree) {
    let (fb, fa) = (before.folder_map(), after.folder_map());
    let (sb, sa) = (before.session_map(), after.session_map());
    let mut frames = Vec::new();
    for (id, f) in sorted(&fb) {
        if !fa.contains_key(id) {
            frames.push(json!({ "op": "folder.deleted", "folder": { "id": id } }));
        } else if fa.get(id) != Some(f) {
            frames.push(json!({ "op": "folder.updated", "folder": fa[id] }));
        }
    }
    for (id, f) in sorted(&fa) {
        if !fb.contains_key(id) {
            frames.push(json!({ "op": "folder.created", "folder": f }));
        }
    }
    for (id, s) in sorted(&sb) {
        match sa.get(id) {
            None => frames.push(json!({ "op": "session.deleted", "session": { "id": id } })),
            Some(now) if now != s && changed_only_unread(s, now) => {
                frames.push(json!({ "op": "unread", "session_id": id, "unread": now["unread"] }));
            }
            Some(now) if now != s => {
                crate::chat_msgs::emit(shared, *id, "session.updated", json!({ "session": now }));
                frames.push(json!({ "op": "session.updated", "session": now }))
            }
            Some(_) => {}
        }
    }
    for (id, s) in sorted(&sa) {
        if !sb.contains_key(id) {
            frames.push(json!({ "op": "session.created", "session": s }));
        }
    }
    if frames.is_empty() {
        return;
    }
    shared.hub.set_state("sessions", after.snapshot());
    for data in frames {
        shared.hub.emit_keep("sessions", FrameType::Delta, data);
    }
}
