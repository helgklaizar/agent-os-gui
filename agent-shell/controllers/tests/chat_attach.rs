//! S4-10: attachments: choice via a dialog, preview of name and size, sending with the message.

mod chat_common;

use chat_common::*;
use controllers::chat_types::AttachmentRef;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{Controller, CoreCommand};
use serde_json::json;

fn pick(c: &mut ChatController, path: &str, name: &str, size: u64) {
    step(
        c,
        ChatMsg::AttachPicked {
            path: path.into(),
            name: name.into(),
            size,
        },
    );
}

fn previews(c: &ChatController) -> Vec<(String, String)> {
    c.view()
        .attachments
        .into_iter()
        .map(|a| (a.name, a.size_text))
        .collect()
}

#[test]
fn attachment_preview_name() {
    let mut c = open(None, vec![]);
    pick(&mut c, "/tmp/secret/a.pdf", "a.pdf", 1536);
    pick(&mut c, "/tmp/b", "b.bin", 12);
    pick(&mut c, "/tmp/c", "c.iso", 5 * 1024 * 1024);
    assert_eq!(
        previews(&c),
        [
            ("a.pdf".into(), "1.5 KB".into()),
            ("b.bin".into(), "12 B".into()),
            ("c.iso".into(), "5.0 MB".into())
        ]
    );
    let shown = format!("{:?}", c.view().attachments);
    assert!(
        !shown.contains("secret"),
        "the path does not reach the preview"
    );
}

#[test]
fn attach_dialog_flag() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::AttachClicked);
    assert!(c.view().want_attach_dialog);
    pick(&mut c, "/tmp/a", "a", 1);
    assert!(!c.view().want_attach_dialog);
    step(&mut c, ChatMsg::AttachClicked);
    step(&mut c, ChatMsg::AttachDialogClosed);
    assert!(!c.view().want_attach_dialog && c.view().attachments.len() == 1);
    let mut none = ChatController::new();
    step(&mut none, ChatMsg::AttachClicked);
    assert!(
        !none.view().want_attach_dialog,
        "without a session there is nowhere to attach"
    );
}

#[test]
fn attachment_goes_with_message() {
    let mut c = open(None, vec![]);
    pick(&mut c, "/tmp/a.pdf", "a.pdf", 1536);
    step(&mut c, ChatMsg::DraftChanged("see the file".into()));
    let fx = step(
        &mut c,
        ChatMsg::Send {
            client_id: "cid".into(),
        },
    );
    let want = AttachmentRef {
        path: "/tmp/a.pdf".into(),
        name: "a.pdf".into(),
        size: 1536,
    };
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::SendMessage {
            session_id: SID,
            client_id: "cid".into(),
            body_md: "see the file".into(),
            attachments: vec![want],
        }]
    );
    assert!(
        c.view().attachments.is_empty(),
        "no attachments in the draft after sending"
    );
}

#[test]
fn attachment_alone_is_sendable() {
    let mut c = open(None, vec![]);
    pick(&mut c, "/tmp/a", "a", 1);
    assert!(c.view().can_send);
    assert_eq!(
        commands(&step(
            &mut c,
            ChatMsg::Send {
                client_id: "x".into()
            }
        ))
        .len(),
        1
    );
}

#[test]
fn attach_remove_and_duplicate_path() {
    let mut c = open(None, vec![]);
    pick(&mut c, "/tmp/a", "a", 1);
    pick(&mut c, "/tmp/a", "a", 1);
    pick(&mut c, "/tmp/b", "b", 2);
    assert_eq!(previews(&c).len(), 2, "the same file is not attached twice");
    step(&mut c, ChatMsg::AttachRemove(0));
    assert_eq!(previews(&c), [("b".into(), "2 B".into())]);
    step(&mut c, ChatMsg::AttachRemove(9));
    assert_eq!(previews(&c).len(), 1);
}

#[test]
fn attachment_name_is_plain_text_without_controls() {
    let mut c = open(None, vec![]);
    pick(&mut c, "/tmp/x", "<b>x</b>\u{202E}fdp.exe\n\r", 3);
    assert_eq!(
        previews(&c)[0].0,
        "<b>x</b>fdp.exe",
        "markup is literal, controls and RTL spoofing are removed"
    );
}

#[test]
fn message_attachments_shown_by_name_only() {
    let mut m = msg_json(1, "user", "here", "done");
    m["meta"] = json!({ "attachments": [
        { "id": 3, "name": "<img src=x onerror=alert(1)>.txt", "size": 2048, "mime": "text/plain" },
        { "id": 4, "name": "ok.png", "size": 10, "mime": "image/png" }
    ]});
    let c = open(None, vec![m]);
    let v = c.view();
    let b = bubbles(&v);
    let names: Vec<_> = b[0]
        .attachments
        .iter()
        .map(|a| (a.name.as_str(), a.size_text.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("<img src=x onerror=alert(1)>.txt", "2.0 KB"),
            ("ok.png", "10 B")
        ]
    );
    assert_eq!(plain(b[0]), "here", "file contents do not reach the bubble");
}
