//! S4-12: escaping and secrets in the chat: user and agent text is shown literally.

mod chat_common;

use std::time::{Duration, Instant};

use chat_common::*;
use controllers::Controller;
use controllers::chat_types::AttachmentRef;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::i18n::{tr, tr_args};
use controllers::markdown::{Block, Inline};
use controllers::secrets::mask_secrets;
use controllers::testkit::evil_strings;
use controllers::text::escape_markup;
use controllers::tree_types::PersonaItem;
use serde_json::json;

fn created(m: serde_json::Value) -> serde_json::Value {
    event("message.created", json!({ "message": m }))
}

#[test]
fn escape_message_body() {
    let mut c = open(None, vec![]);
    let evil = "<img src=x onerror=alert(1)>";
    step(&mut c, ChatMsg::DraftChanged(evil.into()));
    step(
        &mut c,
        ChatMsg::Send {
            client_id: "c1".into(),
        },
    );
    let v = c.view();
    let b = bubbles(&v);
    assert_eq!(
        b[0].blocks,
        vec![Block::Paragraph(vec![Inline::Text(evil.into())])]
    );
    assert_eq!(
        escape_markup(&plain(b[0])),
        "&lt;img src=x onerror=alert(1)&gt;"
    );
}

#[test]
fn escape_agent_echo() {
    let mut c = open(None, vec![]);
    let echo = "echo: <script>alert(1)</script> <b onmouseover=x>";
    frame(&mut c, false, created(msg_json(3, "agent", echo, "done")));
    let v = c.view();
    assert_eq!(
        plain(bubbles(&v)[0]),
        echo,
        "the agent echo is not turned into markup"
    );
    assert!(!escape_markup(&plain(bubbles(&v)[0])).contains('<'));
}

#[test]
fn escape_author_and_attachment_name() {
    let evil_persona = PersonaItem {
        id: 1,
        name: "\" onmouseover=\"x<b>".into(),
        color: "#fff".into(),
    };
    let mut c = ChatController::new();
    step(&mut c, ChatMsg::Personas(vec![evil_persona]));
    step(&mut c, ChatMsg::Select(Some(SID)));
    frame(
        &mut c,
        true,
        snapshot_data(Some(1), vec![msg_json(1, "agent", "x", "done")], vec![]),
    );
    step(
        &mut c,
        ChatMsg::AttachPicked {
            path: "/p".into(),
            name: "\"><script>.txt".into(),
            size: 1,
        },
    );
    let v = c.view();
    assert_eq!(
        bubbles(&v)[0].author_line,
        "Agent · \" onmouseover=\"x<b>",
        "the caption is as is, the view escapes it"
    );
    assert_eq!(v.attachments[0].name, "\"><script>.txt");
    for text in [&bubbles(&v)[0].author_line, &v.attachments[0].name] {
        let e = escape_markup(text);
        assert!(
            !e.contains(['<', '>', '"']),
            "only escaped text goes into Pango markup: {e}"
        );
    }
    assert_eq!(v.chips.unwrap().persona_label, "\" onmouseover=\"x<b>");
}

#[test]
fn escape_long_message_100k() {
    let mut c = open(None, vec![]);
    let start = Instant::now();
    for evil in evil_strings() {
        frame(&mut c, false, created(msg_json(50, "agent", &evil, "done")));
        let _ = c.view();
    }
    let long = "<".repeat(100_000);
    step(&mut c, ChatMsg::DraftChanged(long.clone()));
    step(
        &mut c,
        ChatMsg::Send {
            client_id: "big".into(),
        },
    );
    let v = c.view();
    assert!(bubbles(&v).iter().any(|b| plain(b).len() >= 100_000));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "100k characters processed in {:?}",
        start.elapsed()
    );
}

#[test]
fn escape_pending_attachment_name_in_send_command() {
    let mut c = open(None, vec![]);
    step(
        &mut c,
        ChatMsg::AttachPicked {
            path: "/p".into(),
            name: "a\u{202E}b".into(),
            size: 1,
        },
    );
    step(&mut c, ChatMsg::DraftChanged("x".into()));
    let fx = step(
        &mut c,
        ChatMsg::Send {
            client_id: "i".into(),
        },
    );
    let sent = commands(&fx);
    let controllers::CoreCommand::SendMessage { attachments, .. } = sent[0] else {
        panic!("no SendMessage")
    };
    assert_eq!(
        attachments,
        &vec![AttachmentRef {
            path: "/p".into(),
            name: "ab".into(),
            size: 1
        }]
    );
}

#[test]
fn secrets_masked_in_log_args() {
    let tok = format!("agt_{}", "Z".repeat(43));
    let cases = [
        format!("curl -H 'Authorization: Bearer {tok}' https://x"),
        format!("token={tok}"),
        format!("{{\"api_key\":\"{tok}\",\"x\":1}}"),
        "password: hunter2 and more text".to_string(),
        "SECRET=\"abc123\" run".to_string(),
        format!("key {tok}"),
        format!("sk-{} in text", "q".repeat(30)),
    ];
    for src in cases {
        let masked = mask_secrets(&src);
        assert!(masked.contains("***"), "no mask: {masked}");
        for leak in [
            tok.as_str(),
            "hunter2",
            "abc123",
            &"q".repeat(30),
            &"Z".repeat(20),
        ] {
            assert!(!masked.contains(leak), "secret {leak:?} remains: {masked}");
        }
    }
    assert_eq!(
        mask_secrets("ls -la /tmp  and spaces"),
        "ls -la /tmp  and spaces",
        "plain text is left alone"
    );
    assert_eq!(mask_secrets("token=abc, next"), "token=***, next");
}

#[test]
fn i18n_chat_strings_from_ftl() {
    for (key, want) in [
        ("chat-empty", "Create a session on the left"),
        ("chat-typing", "the agent is typing…"),
        ("chat-btn-allow", "Allow"),
        ("chat-btn-deny", "Deny"),
        ("chat-btn-always", "Always"),
        ("chat-btn-confirm", "Confirm"),
        ("chat-interrupted", "reply interrupted"),
        ("chat-error-title", "No reply received"),
    ] {
        assert_eq!(tr(key), want, "{key}");
    }
    assert_eq!(
        tr_args("chat-author-agent", &[("persona", "Work")]),
        "Agent · Work"
    );
    let c = open_with_card();
    let v = c.view();
    let card = asks(&v)[0];
    let labels: Vec<_> = card.buttons.iter().map(|b| b.label.clone()).collect();
    assert_eq!(
        labels,
        [
            tr("chat-btn-allow"),
            tr("chat-btn-deny"),
            tr("chat-btn-always")
        ],
        "card captions from en.ftl"
    );
    assert!(!card.title.contains('⟦'), "key found");
}
