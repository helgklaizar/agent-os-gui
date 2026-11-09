//! "Chat" column: widget assembly and signals; applying the view is in `chat_apply`.
//! The stream shows the last `WINDOW` items, earlier ones via the "Show earlier" button (5000 messages are not built at once).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use controllers::chat_view::ChatItem;
use controllers::controllers::chat::{ChatMsg, InputKey};
use controllers::i18n::tr;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::chat_items::{Sender, plain};

/// How many stream items are built at once.
pub(crate) const WINDOW: usize = 300;

#[derive(Default)]
pub(crate) struct Shown {
    pub(crate) session: Option<i64>,
    pub(crate) start: usize,
    pub(crate) widgets: Vec<(ChatItem, gtk::Widget)>,
    pub(crate) scroll_seq: u64,
}

pub struct ChatWidgets {
    pub root: gtk::Box,
    pub input: gtk::TextView,
    pub send_btn: gtk::Button,
    pub stop_btn: gtk::Button,
    pub(crate) attach_btn: gtk::Button,
    pub(crate) persona_chip: gtk::Button,
    pub(crate) path_chip: gtk::Button,
    pub(crate) empty: gtk::Label,
    pub(crate) main: gtk::Box,
    pub(crate) persona_list: gtk::Box,
    pub(crate) chips: gtk::Box,
    pub(crate) items: gtk::Box,
    pub(crate) earlier: gtk::Button,
    pub(crate) scroller: gtk::ScrolledWindow,
    pub(crate) typing: gtk::Label,
    pub(crate) attachments: gtk::Box,
    pub(crate) shown: RefCell<Shown>,
    pub(crate) quiet: Cell<bool>,
    pub(crate) window: Cell<usize>,
    pub(crate) send: Sender,
}

pub(crate) fn flat(label: &str, css: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("shell-btn");
    b.add_css_class(css);
    a11y::set_name(&b, label);
    b
}

impl ChatWidgets {
    pub fn new(send: Sender) -> Rc<ChatWidgets> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let empty = plain("empty-hint", &tr("chat-empty"));
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_vexpand(true);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("chat-chips");
        let persona_chip = gtk::Button::new();
        let path_chip = gtk::Button::new();
        for (chip, name) in [
            (&persona_chip, tr("chat-chip-persona-name")),
            (&path_chip, tr("chat-chip-path-name")),
        ] {
            chip.add_css_class("shell-btn");
            chip.add_css_class("chat-chip");
            a11y::set_name(chip, &name);
            chips.append(chip);
        }
        let persona_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        persona_list.add_css_class("chat-persona-list");
        persona_list.set_visible(false);
        let items = gtk::Box::new(gtk::Orientation::Vertical, 8);
        items.add_css_class("chat-items");
        let earlier = flat(&tr("chat-show-earlier"), "chat-earlier");
        earlier.set_visible(false);
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&earlier);
        inner.append(&items);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&inner)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        a11y::set_name(&scroller, &tr("chat-messages-name"));
        let typing = plain("chat-typing", "");
        typing.set_visible(false);
        let attachments = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let input = gtk::TextView::new();
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.add_css_class("chat-input");
        a11y::set_name(&input, &tr("chat-input-name"));
        let input_scroll = gtk::ScrolledWindow::builder()
            .child(&input)
            .min_content_height(40)
            .max_content_height(120)
            .propagate_natural_height(true)
            .hexpand(true)
            .build();
        let attach_btn = a11y::icon_button("mail-attachment-symbolic", &tr("chat-attach"));
        let send_btn = flat(&tr("chat-send"), "card-btn-ok");
        let stop_btn = flat(&tr("chat-stop"), "card-btn-deny");
        stop_btn.set_visible(false);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("chat-compose");
        attach_btn.add_css_class("shell-btn");
        for w in [
            attach_btn.upcast_ref::<gtk::Widget>(),
            input_scroll.upcast_ref(),
            send_btn.upcast_ref(),
            stop_btn.upcast_ref(),
        ] {
            bar.append(w);
        }
        for w in [
            chips.upcast_ref::<gtk::Widget>(),
            persona_list.upcast_ref(),
            scroller.upcast_ref(),
            typing.upcast_ref(),
            attachments.upcast_ref(),
            bar.upcast_ref(),
        ] {
            main.append(w);
        }
        root.append(&empty);
        root.append(&main);
        let w = Rc::new(ChatWidgets {
            root,
            input,
            send_btn,
            stop_btn,
            attach_btn,
            persona_chip,
            path_chip,
            empty,
            main,
            persona_list,
            chips,
            items,
            earlier,
            scroller,
            typing,
            attachments,
            shown: RefCell::new(Shown::default()),
            quiet: Cell::new(false),
            window: Cell::new(WINDOW),
            send,
        });
        w.connect();
        w
    }

    fn connect(self: &Rc<Self>) {
        let send = self.send.clone();
        self.input.buffer().connect_changed({
            let me = Rc::downgrade(self);
            move |b| {
                let Some(me) = me.upgrade() else { return };
                if !me.quiet.get() {
                    let text = b.text(&b.start_iter(), &b.end_iter(), false);
                    send(ChatMsg::DraftChanged(text.to_string()));
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let send = self.send.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            if enter && !state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                send(ChatMsg::Key {
                    key: InputKey::Enter,
                    client_id: uuid::Uuid::now_v7().to_string(),
                });
                return gtk::glib::Propagation::Stop;
            }
            // Shift+Enter: the field inserts the line break itself, `DraftChanged` comes from the buffer
            gtk::glib::Propagation::Proceed
        });
        self.input.add_controller(keys);
        self.hook(&self.send_btn, |_| ChatMsg::Send {
            client_id: uuid::Uuid::now_v7().to_string(),
        });
        self.hook(&self.stop_btn, |_| ChatMsg::Stop);
        self.hook(&self.attach_btn, |_| ChatMsg::AttachClicked);
        self.hook(&self.persona_chip, |_| ChatMsg::PersonaChipClicked);
        self.hook(&self.path_chip, |_| ChatMsg::PathChipClicked);
        let me = Rc::downgrade(self);
        self.earlier.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.show_earlier();
            }
        });
        let send = self.send.clone();
        self.scroller.vadjustment().connect_value_changed(move |a| {
            let at_bottom = a.value() >= a.upper() - a.page_size() - 8.0;
            send(ChatMsg::Scrolled { at_bottom });
        });
    }

    /// The stream is wider than the column (horizontal scroll): for checking the minimum window size.
    pub fn horizontal_overflow(&self) -> bool {
        let a = self.scroller.hadjustment();
        a.upper() > a.page_size() + 1.0
    }

    fn hook(&self, b: &gtk::Button, make: impl Fn(()) -> ChatMsg + 'static) {
        let send = self.send.clone();
        b.connect_clicked(move |_| send(make(())));
    }

    fn show_earlier(self: &Rc<Self>) {
        self.window.set(self.window.get() + WINDOW);
        self.shown.borrow_mut().start = usize::MAX;
        // any message to the controller makes the worker layer apply the view again
        (self.send)(ChatMsg::Scrolled { at_bottom: false });
    }
}
