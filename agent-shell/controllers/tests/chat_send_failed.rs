//! S4-fix: a core refusal to send reaches the chat: the placeholder bubble gets the "not sent" status.

mod chat_common;

use chat_common::*;
use controllers::Controller;
use controllers::chat_view::Delivery;
use controllers::controllers::chat::{ChatMsg, InputKey};

fn send(c: &mut controllers::controllers::chat::ChatController, id: &str, text: &str) {
    step(c, ChatMsg::DraftChanged(text.into()));
    step(
        c,
        ChatMsg::Key {
            key: InputKey::Enter,
            client_id: id.into(),
        },
    );
}

#[test]
fn send_failed_marks_pending_bubble() {
    let mut c = open(None, vec![]);
    send(&mut c, "cid-1", "hello");
    assert_eq!(bubbles(&c.view())[0].delivery, Delivery::Pending);
    step(
        &mut c,
        ChatMsg::SendFailed {
            client_id: "cid-1".into(),
        },
    );
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(b.len(), 1, "the bubble did not vanish: {:?}", v.items);
    assert_eq!(b[0].delivery, Delivery::Failed);
    assert!(b[0].note.is_some(), "the bubble has a failure note");
    assert_eq!(plain(b[0]), "hello");
}

#[test]
fn send_failed_unknown_client_id_is_ignored() {
    let mut c = open(None, vec![]);
    send(&mut c, "cid-1", "hello");
    step(
        &mut c,
        ChatMsg::SendFailed {
            client_id: "other".into(),
        },
    );
    assert_eq!(bubbles(&c.view())[0].delivery, Delivery::Pending);
}
