//! Shared chat test helpers: session fixture, frame JSON, sending messages to the controller.
#![allow(dead_code)]

use std::time::Instant;

use controllers::CoreCommand;
use controllers::chat_view::{Bubble, ChatItem, ChatView};
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::permissions::{PermDecision, RuleSpec};
use controllers::tree_types::PersonaItem;
use controllers::{Controller, Effect};
use serde_json::{Value, json};

pub const SID: i64 = 7;
pub const WORK: i64 = 10;
pub const HOME: i64 = 11;
/// 2026-11-03 12:34:56 UTC.
pub const TS: i64 = 1_793_450_096;

pub fn personas() -> Vec<PersonaItem> {
    vec![
        PersonaItem {
            id: WORK,
            name: "Work".into(),
            color: "#1c7ed6".into(),
        },
        PersonaItem {
            id: HOME,
            name: "Personal".into(),
            color: "#e64980".into(),
        },
    ]
}

pub fn msg_json(id: i64, role: &str, body: &str, status: &str) -> Value {
    json!({ "id": id, "session_id": SID, "client_id": null, "role": role, "author": null,
            "body_md": body, "status": status, "meta": null, "ts": TS })
}

pub fn snapshot_data(persona: Option<i64>, messages: Vec<Value>, open: Vec<Value>) -> Value {
    json!({ "session": { "id": SID, "persona_id": persona, "effective_persona_id": persona, "path": "/home/u/code" },
            "messages": messages, "open_requests": open, "tasks": [] })
}

pub fn event(kind: &str, mut data: Value) -> Value {
    data["kind"] = json!(kind);
    data
}

pub fn step(c: &mut ChatController, msg: ChatMsg) -> Vec<Effect> {
    c.update(msg, Instant::now())
}

pub fn frame(c: &mut ChatController, snapshot: bool, data: Value) -> Vec<Effect> {
    step(
        c,
        ChatMsg::Frame {
            session_id: SID,
            snapshot,
            data,
        },
    )
}

/// A controller with personas, the selected session SID and a snapshot made of `messages`.
pub fn open(persona: Option<i64>, messages: Vec<Value>) -> ChatController {
    let mut c = ChatController::new();
    step(&mut c, ChatMsg::Personas(personas()));
    step(&mut c, ChatMsg::Select(Some(SID)));
    frame(&mut c, true, snapshot_data(persona, messages, vec![]));
    c
}

pub fn bubbles(v: &ChatView) -> Vec<&Bubble> {
    v.items
        .iter()
        .filter_map(|i| match i {
            ChatItem::Bubble(b) => Some(b),
            _ => None,
        })
        .collect()
}

/// Bubble text: paragraphs, code and links in a row, without markup.
pub fn plain(b: &Bubble) -> String {
    use controllers::markdown::{Block, Inline};
    let inl = |i: &Inline| match i {
        Inline::Text(s) | Inline::Code(s) | Inline::Strong(s) | Inline::Emph(s) => s.clone(),
        Inline::Link { text, .. } => text.clone(),
    };
    b.blocks
        .iter()
        .map(|bl| match bl {
            Block::Paragraph(i) | Block::Heading { inlines: i, .. } => {
                i.iter().map(inl).collect::<String>()
            }
            Block::Code { text, .. } => text.clone(),
            Block::List { items, .. } => items.iter().flatten().map(inl).collect::<String>(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn commands(effects: &[Effect]) -> Vec<&controllers::CoreCommand> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c),
            _ => None,
        })
        .collect()
}

pub const REQ: i64 = 101;

/// `PermissionRequest` per api.md section 6.
pub fn request_json(id: i64, action: &str, target: &str, status: &str) -> Value {
    json!({ "id": id, "session_id": SID, "action": action, "target": target, "source": "policy",
            "level": "ask", "status": status, "detail": null })
}

/// A chat card is a `system` message with `meta.kind = permission_request`.
pub fn card_msg(msg_id: i64, req: &Value) -> Value {
    let mut m = msg_json(msg_id, "system", "", "done");
    m["meta"] = json!({ "kind": "permission_request", "request_id": req["id"], "request": req });
    m
}

/// A session with the Work persona and one pending `send` card.
pub fn open_with_card() -> ChatController {
    let req = request_json(REQ, "send", "email to the client", "pending");
    let mut c = ChatController::new();
    step(&mut c, ChatMsg::Personas(personas()));
    step(&mut c, ChatMsg::Select(Some(SID)));
    frame(
        &mut c,
        true,
        snapshot_data(Some(WORK), vec![card_msg(20, &req)], vec![req]),
    );
    c
}

pub fn asks(v: &ChatView) -> Vec<&controllers::chat_view::AskCard> {
    v.items
        .iter()
        .filter_map(|i| match i {
            ChatItem::Ask(a) => Some(a),
            _ => None,
        })
        .collect()
}

pub fn click(c: &mut ChatController, b: controllers::chat_view::ButtonKind) -> Vec<Effect> {
    step(
        c,
        ChatMsg::CardButton {
            request_id: REQ,
            button: b,
        },
    )
}

pub fn decide(decision: PermDecision, rule: Option<RuleSpec>) -> CoreCommand {
    CoreCommand::DecidePermission {
        request_id: REQ,
        decision,
        rule,
    }
}
