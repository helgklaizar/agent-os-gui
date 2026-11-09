//! S4-03: the message list and input of ChatController.

mod chat_common;

use chat_common::*;
use controllers::chat_types::AttachmentRef;
use controllers::controllers::chat::{ChatController, ChatMsg, InputKey};
use controllers::{Controller, CoreCommand, Effect};

fn commands(effects: &[Effect]) -> Vec<&CoreCommand> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn key(c: &mut ChatController, k: InputKey, id: &str) -> Vec<Effect> {
    step(
        c,
        ChatMsg::Key {
            key: k,
            client_id: id.into(),
        },
    )
}

#[test]
fn send_enter_submits() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::DraftChanged("hello".into()));
    let fx = key(&mut c, InputKey::Enter, "cid-1");
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::SendMessage {
            session_id: SID,
            client_id: "cid-1".into(),
            body_md: "hello".into(),
            attachments: Vec::<AttachmentRef>::new(),
        }]
    );
    assert_eq!(c.view().draft, "", "the input field is empty after sending");
}

#[test]
fn send_shift_enter_newline() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::DraftChanged("a".into()));
    let fx = key(&mut c, InputKey::ShiftEnter, "cid-1");
    assert!(
        commands(&fx).is_empty(),
        "Shift+Enter does not send: {fx:?}"
    );
    assert_eq!(c.view().draft, "a\n");
}

#[test]
fn send_empty_draft_is_ignored() {
    let mut c = open(None, vec![]);
    for text in ["", "  \n "] {
        step(&mut c, ChatMsg::DraftChanged(text.into()));
        let fx = key(&mut c, InputKey::Enter, "cid-1");
        assert!(commands(&fx).is_empty(), "empty input {text:?} is not sent");
    }
    assert!(!c.view().can_send);
}

#[test]
fn send_message_has_client_id() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::DraftChanged("one".into()));
    let a = step(
        &mut c,
        ChatMsg::Send {
            client_id: "id-a".into(),
        },
    );
    step(&mut c, ChatMsg::DraftChanged("two".into()));
    let b = step(
        &mut c,
        ChatMsg::Send {
            client_id: "id-b".into(),
        },
    );
    let ids: Vec<_> = [a, b]
        .iter()
        .flat_map(|fx| {
            commands(fx)
                .into_iter()
                .filter_map(|c| match c {
                    CoreCommand::SendMessage { client_id, .. } => Some(client_id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(ids, ["id-a", "id-b"]);
}

#[test]
fn sent_message_shows_pending_then_dedup_by_client_id() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::DraftChanged("text".into()));
    key(&mut c, InputKey::Enter, "cid-9");
    let v = c.view();
    assert_eq!(
        bubbles(&v).len(),
        1,
        "the own placeholder bubble is visible before the core answers"
    );
    let mut m = msg_json(5, "user", "text", "done");
    m["client_id"] = "cid-9".into();
    frame(
        &mut c,
        false,
        event("message.created", serde_json::json!({ "message": m })),
    );
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(
        b.len(),
        1,
        "no duplicate: the placeholder is replaced by the core message"
    );
    assert_eq!(b[0].id, Some(5));
}

#[test]
fn draft_per_session() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::DraftChanged("draft A".into()));
    step(&mut c, ChatMsg::Select(Some(8)));
    assert_eq!(c.view().draft, "", "session B has no draft");
    step(&mut c, ChatMsg::DraftChanged("draft B".into()));
    step(&mut c, ChatMsg::Select(Some(SID)));
    assert_eq!(c.view().draft, "draft A");
    step(&mut c, ChatMsg::Select(Some(8)));
    assert_eq!(c.view().draft, "draft B");
}

#[test]
fn author_line_format() {
    let mut c = open(Some(WORK), vec![msg_json(1, "agent", "reply", "done")]);
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(b[0].author_line, "Agent · Work");
    assert_eq!(b[0].time, "12:34");
    assert!(!b[0].mine);
    frame(
        &mut c,
        false,
        event(
            "message.created",
            serde_json::json!({ "message": msg_json(2, "user", "me", "done") }),
        ),
    );
    let v = c.view();
    assert!(bubbles(&v)[1].mine, "the user's message is on the right");
}

#[test]
fn no_session_empty_state() {
    let mut c = ChatController::new();
    step(&mut c, ChatMsg::Select(None));
    let v = c.view();
    assert_eq!(v.empty.as_deref(), Some("Create a session on the left"));
    assert!(v.chips.is_none() && v.items.is_empty());
    let fx = key(&mut c, InputKey::Enter, "x");
    assert!(commands(&fx).is_empty());
    step(&mut c, ChatMsg::Select(Some(SID)));
    assert_eq!(c.view().empty, None);
}

fn created(id: i64) -> serde_json::Value {
    event(
        "message.created",
        serde_json::json!({ "message": msg_json(id, "agent", "x", "done") }),
    )
}

#[test]
fn autoscroll_when_at_bottom() {
    let mut c = open(None, vec![msg_json(1, "agent", "a", "done")]);
    let before = c.view().scroll_seq;
    frame(&mut c, false, created(2));
    assert!(
        c.view().scroll_seq > before,
        "at the bottom: the stream scrolls to the new message"
    );
}

#[test]
fn no_autoscroll_when_scrolled_up() {
    let mut c = open(None, vec![msg_json(1, "agent", "a", "done")]);
    step(&mut c, ChatMsg::Scrolled { at_bottom: false });
    let before = c.view().scroll_seq;
    frame(&mut c, false, created(2));
    assert_eq!(
        c.view().scroll_seq,
        before,
        "scrolled up: the scroll does not jump"
    );
    step(&mut c, ChatMsg::Scrolled { at_bottom: true });
    frame(&mut c, false, created(3));
    assert!(
        c.view().scroll_seq > before,
        "back at the bottom: the scroll follows again"
    );
}
