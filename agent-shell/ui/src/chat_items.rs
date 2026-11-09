//! Chat bubbles and cards. All text is plain labels without markup; the persona colour is not used,
//! the border and states are CSS classes from the theme tokens.

use controllers::chat_view::{
    AskCard, Bubble, ButtonKind, CardButton, CardState, ChatItem, Delivery, ErrorCard, ObserverCard,
};
use controllers::controllers::chat::ChatMsg;
use controllers::i18n::tr;
use controllers::permissions::Mode;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::chat_md::block_widget;

pub(crate) type Sender = std::rc::Rc<dyn Fn(ChatMsg)>;

pub(crate) fn plain(css: &str, text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_use_markup(false);
    label.set_xalign(0.0);
    label.set_max_width_chars(70);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.add_css_class(css);
    label
}

fn card_box(role: gtk::AccessibleRole, name: &str, css: &[&str]) -> gtk::Box {
    let b = a11y::landmark(role, gtk::Orientation::Vertical, name);
    b.set_spacing(4);
    b.add_css_class("chat-card");
    for c in css {
        b.add_css_class(c);
    }
    b
}

fn button(label: &str, css: &str, on: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("shell-btn");
    b.add_css_class(css);
    a11y::set_name(&b, label);
    b.connect_clicked(move |_| on());
    b
}

fn bubble_widget(b: &Bubble) -> gtk::Widget {
    let name = format!("{}, {}", b.author_line, b.time);
    let root = a11y::landmark(
        gtk::AccessibleRole::Group,
        gtk::Orientation::Vertical,
        &name,
    );
    root.set_spacing(2);
    root.add_css_class("chat-bubble");
    root.add_css_class(if b.mine { "mine" } else { "theirs" });
    root.set_halign(if b.mine {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    let head = plain("chat-meta", &format!("{}  {}", b.author_line, b.time));
    root.append(&head);
    for block in &b.blocks {
        root.append(&block_widget(block));
    }
    for a in &b.attachments {
        root.append(&plain(
            "chat-attach",
            &format!("📎 {} ({})", a.name, a.size_text),
        ));
    }
    if let Some(note) = &b.note {
        root.append(&plain("chat-note", note));
    }
    match b.delivery {
        Delivery::Pending => root.add_css_class("pending"),
        Delivery::Failed => root.add_css_class("failed"),
        Delivery::Sent => {}
    }
    root.upcast()
}

fn error_widget(c: &ErrorCard) -> gtk::Widget {
    let root = card_box(gtk::AccessibleRole::Alert, &c.title, &["chat-error"]);
    root.append(&plain("chat-card-title", &c.title));
    if !c.text.is_empty() {
        root.append(&plain("chat-p", &c.text));
    }
    root.upcast()
}

fn state_row(root: &gtk::Box, label: &Option<String>) {
    if let Some(l) = label {
        root.append(&plain("chat-card-state", l));
    }
}

fn buttons_row(root: &gtk::Box, buttons: &[CardButton], id: i64, send: &Sender) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for cb in buttons {
        let (send, kind) = (send.clone(), cb.kind);
        let css = if matches!(kind, ButtonKind::Deny | ButtonKind::Reject) {
            "card-btn-deny"
        } else {
            "card-btn-ok"
        };
        row.append(&button(&cb.label, css, move || {
            send(ChatMsg::CardButton {
                request_id: id,
                button: kind,
            });
        }));
    }
    if buttons.iter().any(|_| true) {
        root.append(&row);
    }
}

fn mode_name(m: Mode) -> String {
    match m {
        Mode::Allow => tr("chat-mode-allow"),
        Mode::Ask => tr("chat-mode-ask"),
        Mode::Deny => tr("chat-mode-deny"),
    }
}

fn always_widget(root: &gtk::Box, card: &AskCard, send: &Sender) {
    let Some(chooser) = &card.always else { return };
    for level in &chooser.levels {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&plain("chat-p", &level.label));
        for mode in &level.modes {
            let (send, scope, mode, id) = (send.clone(), level.scope, *mode, card.request_id);
            let label = mode_name(mode);
            row.append(&button(&label, "card-btn-ok", move || {
                send(ChatMsg::AlwaysPick {
                    request_id: id,
                    scope,
                    mode,
                });
            }));
        }
        root.append(&row);
    }
    let (send, id) = (send.clone(), card.request_id);
    root.append(&button(
        &tr("chat-always-cancel"),
        "card-btn-deny",
        move || {
            send(ChatMsg::AlwaysCancel { request_id: id });
        },
    ));
}

fn ask_widget(c: &AskCard, send: &Sender) -> gtk::Widget {
    let root = card_box(gtk::AccessibleRole::Group, &c.title, &["chat-ask"]);
    root.append(&plain("chat-card-title", &c.title));
    root.append(&plain("chat-p", &c.target));
    if let Some(d) = &c.detail {
        root.append(&plain("chat-note", d));
    }
    state_row(&root, &c.state_label);
    if c.state == CardState::Pending {
        buttons_row(&root, &c.buttons, c.request_id, send);
        always_widget(&root, c, send);
    }
    root.upcast()
}

fn observer_widget(c: &ObserverCard, send: &Sender) -> gtk::Widget {
    let name = format!("{}: {}", c.kind_label, c.verdict_label);
    let root = card_box(gtk::AccessibleRole::Alert, &name, &["chat-observer"]);
    root.append(&plain("chat-card-title", &c.kind_label));
    root.append(&plain("chat-p", &c.target));
    root.append(&plain("chat-note", &c.verdict_label));
    if let Some(d) = &c.detail {
        root.append(&plain("chat-note", d));
    }
    state_row(&root, &c.state_label);
    if c.awaiting {
        buttons_row(&root, &c.buttons, c.request_id, send);
    }
    root.upcast()
}

pub(crate) fn item_widget(item: &ChatItem, send: &Sender) -> gtk::Widget {
    match item {
        ChatItem::Bubble(b) => bubble_widget(b),
        ChatItem::Error(c) => error_widget(c),
        ChatItem::Ask(c) => ask_widget(c, send),
        ChatItem::Observer(c) => observer_widget(c, send),
    }
}
