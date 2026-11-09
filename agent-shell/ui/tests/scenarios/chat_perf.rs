//! A chat with 5000 messages opens without a noticeable pause, streaming does not block the GTK main loop.

use std::rc::Rc;
use std::time::{Duration, Instant};

use controllers::chat_view::ChatItem;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{Controller, i18n::tr};
use gtk4 as gtk;
use gtk4::prelude::*;
use serde_json::json;
use ui::chat_widgets::ChatWidgets;

use crate::chat::open_chat;
use crate::ctx::{Ctx, Scenario, WAIT};
use crate::tree::find_all;

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_chat_5000_messages", five_thousand),
        ("ui_chat_stream_does_not_block", stream_not_blocking),
    ]
}

/// Longer than this, opening a chat is a noticeable pause.
const OPEN_LIMIT: Duration = Duration::from_millis(1500);

fn five_thousand(_: &Ctx) -> Result<(), String> {
    let messages: Vec<_> = (1..=5000)
        .map(|i| {
            json!({ "id": i, "session_id": 1, "client_id": null, "role": if i % 2 == 0 { "agent" } else { "user" },
                "author": null, "body_md": format!("Message {i} **bold** `code`"), "status": "done", "meta": null, "ts": 1_793_000_000 + i })
        })
        .collect();
    let mut chat = ChatController::new();
    let _ = chat.update(ChatMsg::Select(Some(1)), Instant::now());
    let started = Instant::now();
    let _ = chat.update(
        ChatMsg::Frame {
            session_id: 1,
            snapshot: true,
            data: json!({ "session": { "id": 1, "name": "x" }, "messages": messages, "open_requests": [], "tasks": [] }),
        },
        Instant::now(),
    );
    let widgets = ChatWidgets::new(Rc::new(|_| {}));
    let view = chat.view();
    widgets.apply(&view);
    let ctx = gtk::glib::MainContext::default();
    while ctx.iteration(false) {}
    let took = started.elapsed();
    let bubbles = view
        .items
        .iter()
        .filter(|i| matches!(i, ChatItem::Bubble(_)))
        .count();
    let built = find_all::<gtk::Label>(widgets.root.upcast_ref()).len();
    println!("    {bubbles} bubbles in the view, caption widgets {built}, opening {took:?}");
    if bubbles != 5000 {
        return Err(format!("{bubbles} bubbles in the view, expected 5000"));
    }
    if built > 3000 {
        return Err(format!(
            "{built} captions built: the stream is not bounded by the window"
        ));
    }
    if took > OPEN_LIMIT {
        return Err(format!(
            "opening the chat took {took:?}, limit {OPEN_LIMIT:?}"
        ));
    }
    if find_all::<gtk::Button>(widgets.root.upcast_ref())
        .iter()
        .all(|b| b.label().is_none_or(|l| l != tr("chat-show-earlier")))
    {
        return Err("no 'Show earlier' button".into());
    }
    Ok(())
}

fn stream_not_blocking(ctx: &Ctx) -> Result<(), String> {
    let rig = open_chat(ctx)?;
    let w = rig.shell.chat_widgets();
    w.input.buffer().set_text("/slow");
    w.send_btn.emit_clicked();
    let main = gtk::glib::MainContext::default();
    let (mut worst, mut last) = (Duration::ZERO, Instant::now());
    let deadline = Instant::now() + WAIT * 2;
    let mut streamed = false;
    while Instant::now() < deadline {
        while main.iteration(false) {}
        let now = Instant::now();
        worst = worst.max(now - last);
        last = now;
        streamed |= rig.shell.chat_view().typing.is_some();
        if streamed && !w.stop_btn.is_visible() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("    worst main loop pause {worst:?}");
    if !streamed {
        return Err("the stream did not start (no 'agent is typing' indicator)".into());
    }
    if worst > Duration::from_millis(300) {
        return Err(format!(
            "the main loop stalled for {worst:?} during the stream"
        ));
    }
    rig.shell.window().destroy();
    Ok(())
}
