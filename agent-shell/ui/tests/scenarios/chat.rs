//! "Chat" column in real GTK under headless Wayland: Enter and Shift+Enter, keyboard, minimum size 1024×640.

use std::time::Duration;

use controllers::chat_view::{ChatItem, ChatView};
use controllers::tree_types::NodeId;
use gtk4 as gtk;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{Rig, find_all, open, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_chat_enter_shift_enter", enter_shift_enter),
        ("ui_chat_keyboard_only", keyboard_only),
        ("ui_chat_1024", at_1024),
    ]
}

/// Selecting a session the way the mouse does: the `pressed` signal of the row's click gesture.
pub(crate) fn select_session(rig: &Rig, name: &str) -> Result<(), String> {
    let Some(NodeId::Session(id)) = rig.node(name) else {
        return Err(format!("no session '{name}'"));
    };
    let row = rig.row_widget(NodeId::Session(id))?;
    let model = row.observe_controllers();
    let click = (0..model.n_items())
        .filter_map(|i| model.item(i))
        .find_map(|o| o.downcast::<gtk::GestureClick>().ok())
        .ok_or("the row has no click gesture")?;
    click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    settle("session selected in chat", WAIT, || {
        rig.shell.chat_view().session_id == Some(id)
    })
}

pub(crate) fn open_chat(ctx: &Ctx) -> Result<Rig, String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    select_session(&rig, "Release 1.2")?;
    Ok(rig)
}

fn key(
    widget: &impl IsA<gtk::Widget>,
    key: gtk::gdk::Key,
    state: gtk::gdk::ModifierType,
) -> Result<bool, String> {
    let model = widget.observe_controllers();
    for i in 0..model.n_items() {
        let Some(obj) = model.item(i) else { continue };
        if let Ok(keys) = obj.downcast::<gtk::EventControllerKey>() {
            return Ok(keys.emit_by_name("key-pressed", &[&key.into_glib(), &0u32, &state]));
        }
    }
    Err("the field has no key controller".into())
}

fn set_text(rig: &Rig, text: &str) {
    rig.shell.chat_widgets().input.buffer().set_text(text);
}

fn has_user_bubble(v: &ChatView, text: &str) -> bool {
    v.items.iter().any(|i| match i {
        ChatItem::Bubble(b) => b.mine && format!("{:?}", b.blocks).contains(text),
        _ => false,
    })
}

fn enter_shift_enter(ctx: &Ctx) -> Result<(), String> {
    let rig = open_chat(ctx)?;
    let w = rig.shell.chat_widgets();
    set_text(&rig, "first");
    let none = gtk::gdk::ModifierType::empty();
    // Shift+Enter is not intercepted (the field inserts the line break) and sends nothing
    if key(
        &w.input,
        gtk::gdk::Key::Return,
        gtk::gdk::ModifierType::SHIFT_MASK,
    )? {
        return Err("Shift+Enter must not be intercepted".into());
    }
    if rig
        .shell
        .chat_view()
        .items
        .iter()
        .any(|i| matches!(i, ChatItem::Bubble(_)))
    {
        return Err("Shift+Enter sent a message".into());
    }
    w.input.buffer().insert_at_cursor("\nsecond");
    settle("draft with a line break", WAIT, || {
        rig.shell.chat_view().draft == "first\nsecond"
    })?;
    if !key(&w.input, gtk::gdk::Key::Return, none)? {
        return Err("Enter must send".into());
    }
    settle("message sent, field empty", WAIT, || {
        let v = rig.shell.chat_view();
        has_user_bubble(&v, "second") && v.draft.is_empty()
    })?;
    let b = w.input.buffer();
    if !b.text(&b.start_iter(), &b.end_iter(), false).is_empty() {
        return Err("the input field was not cleared after Enter".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn find_button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    find_all::<gtk::Button>(root)
        .into_iter()
        .find(|b| b.label().is_some_and(|l| l == label))
}

fn keyboard_only(ctx: &Ctx) -> Result<(), String> {
    let rig = open_chat(ctx)?;
    let w = rig.shell.chat_widgets();
    let root: gtk::Widget = w.root.clone().upcast();
    w.input.grab_focus();
    // stop: appears during the stream, reachable by focus and activation
    set_text(&rig, "/slow");
    key(
        &w.input,
        gtk::gdk::Key::Return,
        gtk::gdk::ModifierType::empty(),
    )?;
    settle("'Stop' button visible", WAIT, || w.stop_btn.is_visible())?;
    if !w.stop_btn.is_focusable() {
        return Err("the 'Stop' button is not focusable".into());
    }
    w.stop_btn.activate();
    settle("reply stopped", WAIT, || !w.stop_btn.is_visible())?;
    // ask card: all buttons are focusable, the answer is a button activation (same as Enter on it)
    set_text(&rig, "/ask send a@example.org");
    key(
        &w.input,
        gtk::gdk::Key::Return,
        gtk::gdk::ModifierType::empty(),
    )?;
    let allow = controllers::i18n::tr("chat-btn-allow");
    settle("ask card with an 'Allow' button", WAIT, || {
        find_button(&root, &allow).is_some()
    })?;
    for label in [
        allow.clone(),
        controllers::i18n::tr("chat-btn-deny"),
        controllers::i18n::tr("chat-btn-always"),
    ] {
        let b = find_button(&root, &label).ok_or(format!("no button '{label}'"))?;
        if !b.is_focusable() || !b.can_target() {
            return Err(format!("button '{label}' is unavailable from the keyboard"));
        }
    }
    let b = find_button(&root, &allow).ok_or("no button")?;
    b.grab_focus();
    if gtk::prelude::GtkWindowExt::focus(rig.shell.window()).as_ref()
        != Some(b.upcast_ref::<gtk::Widget>())
    {
        return Err("focus did not land on the card button".into());
    }
    b.activate();
    settle("card resolved", WAIT, || {
        find_button(&root, &allow).is_none()
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn at_1024(ctx: &Ctx) -> Result<(), String> {
    let rig = open_chat(ctx)?;
    rig.shell.window().set_default_size(1024, 640);
    rig.shell.window().set_size_request(1024, 640);
    let w = rig.shell.chat_widgets();
    let long_word = "LongWordWithoutSpaces".repeat(40);
    set_text(
        &rig,
        &format!("{long_word}\n\n```\n{}\n```", "code ".repeat(200)),
    );
    key(
        &w.input,
        gtk::gdk::Key::Return,
        gtk::gdk::ModifierType::empty(),
    )?;
    settle("message in the stream", WAIT, || {
        has_user_bubble(&rig.shell.chat_view(), "LongWord")
    })?;
    settle("size applied", Duration::from_secs(2), || {
        w.root.width() > 0
    })
    .ok();
    std::thread::sleep(Duration::from_millis(300));
    while gtk::glib::MainContext::default().iteration(false) {}
    if w.horizontal_overflow() {
        return Err("a horizontal scroll appeared in the chat at 1024×640".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
