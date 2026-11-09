//! S4-05: streaming by `offset`, the indicator, stop, an error and an interrupted reply.

mod chat_common;

use chat_common::*;
use controllers::chat_view::ChatItem;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{Controller, CoreCommand};
use serde_json::json;

fn created(id: i64, body: &str, status: &str) -> serde_json::Value {
    event(
        "message.created",
        json!({ "message": msg_json(id, "agent", body, status) }),
    )
}

fn delta(id: i64, offset: usize, text: &str) -> serde_json::Value {
    event(
        "message.delta",
        json!({ "message_id": id, "offset": offset, "text": text }),
    )
}

fn done(id: i64, status: &str, len: usize) -> serde_json::Value {
    event(
        "message.done",
        json!({ "message_id": id, "status": status, "final_len": len }),
    )
}

fn only_bubble_text(c: &ChatController) -> String {
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(b.len(), 1, "one message, not three");
    plain(b[0])
}

#[test]
fn stream_chunks_assemble() {
    let mut c = open(None, vec![]);
    frame(&mut c, false, created(5, "", "streaming"));
    for (off, text) in [(0, "hé"), (3, "llo"), (6, " wörld")] {
        frame(&mut c, false, delta(5, off, text));
    }
    assert_eq!(only_bubble_text(&c), "héllo wörld");
    assert!(bubbles(&c.view())[0].streaming);
    frame(&mut c, false, done(5, "done", "héllo wörld".len()));
    assert_eq!(only_bubble_text(&c), "héllo wörld");
    assert!(!bubbles(&c.view())[0].streaming);
}

#[test]
fn typing_indicator_while_streaming() {
    let mut c = open(None, vec![]);
    assert_eq!(c.view().typing, None);
    frame(&mut c, false, created(5, "é", "streaming"));
    assert_eq!(c.view().typing.as_deref(), Some("the agent is typing…"));
    frame(&mut c, false, done(5, "done", 2));
    assert_eq!(c.view().typing, None);
    frame(&mut c, false, event("typing", json!({ "on": true })));
    assert!(
        c.view().typing.is_some(),
        "a typing frame turns the indicator on"
    );
    frame(&mut c, false, event("typing", json!({ "on": false })));
    assert_eq!(c.view().typing, None);
}

#[test]
fn stop_sends_command() {
    let mut c = open(None, vec![]);
    assert!(
        commands(&step(&mut c, ChatMsg::Stop)).is_empty(),
        "nothing to stop"
    );
    frame(&mut c, false, created(5, "çà", "streaming"));
    assert!(c.view().can_stop);
    let fx = step(&mut c, ChatMsg::Stop);
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::StopStream { session_id: SID }]
    );
    frame(&mut c, false, done(5, "stopped", 4));
    let v = c.view();
    assert!(!v.can_stop && v.typing.is_none());
    let b = bubbles(&v);
    assert_eq!(plain(b[0]), "çà", "the partial text is kept");
    assert_eq!(b[0].note.as_deref(), Some("reply stopped"));
}

#[test]
fn stream_error_card() {
    let mut c = open(None, vec![]);
    frame(&mut c, false, created(5, "part", "streaming"));
    frame(&mut c, false, done(5, "error", "part".len()));
    let v = c.view();
    let card = v.items.iter().find_map(|i| match i {
        ChatItem::Error(e) => Some(e),
        _ => None,
    });
    let card = card.expect("error card");
    assert_eq!(
        (card.message_id, card.title.as_str(), card.text.as_str()),
        (5, "No reply received", "part")
    );
    assert!(bubbles(&v).is_empty(), "a card instead of a bubble");
    step(&mut c, ChatMsg::DraftChanged("next".into()));
    let fx = step(
        &mut c,
        ChatMsg::Send {
            client_id: "n".into(),
        },
    );
    assert_eq!(
        commands(&fx).len(),
        1,
        "the interface is responsive: sending works"
    );
}

#[test]
fn stream_interrupted_card() {
    let c = open(None, vec![msg_json(5, "agent", "cut off", "interrupted")]);
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(b[0].note.as_deref(), Some("reply interrupted"));
    assert!(
        !b[0].streaming && v.typing.is_none(),
        "after a core crash it does not \"type\" forever"
    );
}

#[test]
fn stream_delta_offset_applied_only_when_aligned() {
    let mut c = open(None, vec![]);
    frame(&mut c, false, created(5, "", "streaming"));
    assert!(commands(&frame(&mut c, false, delta(5, 0, "hé"))).is_empty());
    let fx = frame(&mut c, false, delta(5, 2, "llo"));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::ResyncSession { session_id: SID }]
    );
    assert_eq!(
        only_bubble_text(&c),
        "hé",
        "a delta with a wrong offset is not applied"
    );
    let again = frame(&mut c, false, delta(5, 9, "x"));
    assert!(
        commands(&again).is_empty(),
        "a snapshot is already requested, not asking a second time"
    );
    frame(
        &mut c,
        true,
        snapshot_data(
            None,
            vec![msg_json(5, "agent", "héllo", "streaming")],
            vec![],
        ),
    );
    assert!(commands(&frame(&mut c, false, delta(5, 6, "!"))).is_empty());
    assert_eq!(only_bubble_text(&c), "héllo!");
}

#[test]
fn stream_delta_for_unknown_message_resyncs() {
    let mut c = open(None, vec![]);
    let fx = frame(&mut c, false, delta(99, 0, "x"));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::ResyncSession { session_id: SID }]
    );
}

#[test]
fn stream_done_with_wrong_final_len_resyncs() {
    let mut c = open(None, vec![]);
    frame(&mut c, false, created(5, "", "streaming"));
    frame(&mut c, false, delta(5, 0, "éè"));
    let fx = frame(&mut c, false, done(5, "done", 40));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::ResyncSession { session_id: SID }]
    );
}

#[test]
fn stream_frames_of_other_session_are_ignored() {
    let mut c = open(None, vec![]);
    let fx = step(
        &mut c,
        ChatMsg::Frame {
            session_id: 99,
            snapshot: false,
            data: created(5, "x", "done"),
        },
    );
    assert!(fx.is_empty() && c.view().items.is_empty());
}

/// S4-real: the core crashed mid-reply, the `typing:false` frame was lost; a new snapshot after reconnecting clears the indicator.
#[test]
fn snapshot_after_reconnect_clears_typing() {
    let mut c = open(None, vec![]);
    frame(&mut c, false, event("typing", json!({ "on": true })));
    assert!(c.view().typing.is_some());
    frame(
        &mut c,
        true,
        snapshot_data(
            None,
            vec![msg_json(5, "agent", "cut off", "interrupted")],
            vec![],
        ),
    );
    assert_eq!(c.view().typing, None, "the indicator does not stick");
}
