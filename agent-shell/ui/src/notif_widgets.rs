//! "Notifications" column widgets: filter panel with counters, card list, collapsed-column strip.
//! No logic here: filters, cards, badges and captions arrive ready-made from `NotifView`; user text
//! is shown as plain text (no Pango markup).

use std::cell::RefCell;
use std::rc::Rc;

use controllers::controllers::notifications::NotifMsg;
use controllers::notif_view::{Accent, AllChip, Badge, EventCard, KindChip, NotifView, StripItem};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

pub(crate) type Sender = Rc<dyn Fn(NotifMsg)>;

/// How many cards are drawn at once (the controller keeps up to 500, older ones via core pages).
const MAX_CARDS: usize = 200;
const FALLBACK_ICON: &str = "emblem-system-symbolic";

pub struct NotifWidgets {
    /// Column content: filters with a list or the collapsed-column strip.
    pub root: gtk::Box,
    expanded: gtk::Box,
    filters: gtk::Box,
    subs: gtk::Box,
    subs_row: gtk::ScrolledWindow,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Label,
    strip: gtk::Box,
    last_filters: RefCell<Option<(AllChip, Vec<KindChip>)>>,
    last_cards: RefCell<Option<Vec<EventCard>>>,
    last_strip: RefCell<Option<Vec<StripItem>>>,
    /// Key of the item that had focus: after a rebuild focus returns to it.
    focus_key: Rc<RefCell<Option<String>>>,
    keyed: RefCell<Vec<(String, gtk::Widget)>>,
    send: Sender,
}

/// How many lines of text a card shows; the full text stays in the label (and in the accessibility tree).
const CARD_TEXT_LINES: i32 = 8;

fn plain(css: &str, text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_use_markup(false);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.add_css_class(css);
    label
}

fn icon(name: Option<&str>) -> gtk::Image {
    gtk::Image::from_icon_name(name.unwrap_or(FALLBACK_ICON))
}

fn badge_label(badge: &Badge) -> gtk::Label {
    let label = gtk::Label::new(badge.text.as_deref());
    label.set_use_markup(false);
    label.add_css_class("tree-badge");
    label.set_visible(badge.text.is_some());
    label
}

fn card_class(accent: Accent) -> &'static str {
    match accent {
        Accent::Ai => "notif-ai",
        Accent::Task => "notif-task",
        Accent::Msg => "notif-msg",
    }
}

impl NotifWidgets {
    pub fn new(send: Sender) -> Rc<NotifWidgets> {
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        filters.add_css_class("notif-filters");
        let subs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        subs.add_css_class("notif-filters");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_accessible_role(gtk::AccessibleRole::List);
        let empty = plain("empty-hint", "");
        empty.set_margin_top(16);
        empty.set_margin_start(16);
        empty.set_margin_end(16);
        empty.set_visible(false);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let expanded = gtk::Box::new(gtk::Orientation::Vertical, 0);
        expanded.set_vexpand(true);
        // filter rows sit in a scroll without bars: their natural width must not widen the column
        let row = |inner: &gtk::Box| {
            gtk::ScrolledWindow::builder()
                .child(inner)
                .hscrollbar_policy(gtk::PolicyType::External)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .build()
        };
        let subs_row = row(&subs);
        expanded.append(&row(&filters));
        expanded.append(&subs_row);
        expanded.append(&empty);
        expanded.append(&scroller);
        let strip = gtk::Box::new(gtk::Orientation::Vertical, 6);
        strip.add_css_class("tree-strip");
        strip.set_halign(gtk::Align::Center);
        strip.set_visible(false);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        root.append(&expanded);
        root.append(&strip);
        Rc::new(NotifWidgets {
            root,
            expanded,
            filters,
            subs,
            subs_row,
            list,
            scroller,
            empty,
            strip,
            last_filters: RefCell::new(None),
            last_cards: RefCell::new(None),
            last_strip: RefCell::new(None),
            focus_key: Rc::new(RefCell::new(None)),
            keyed: RefCell::new(Vec::new()),
            send,
        })
    }

    /// Remembers which element has focus so it can be restored after a rebuild.
    fn track(self: &Rc<Self>, key: &str, widget: &impl IsA<gtk::Widget>) {
        let controller = gtk::EventControllerFocus::new();
        let (slot, k) = (self.focus_key.clone(), key.to_string());
        controller.connect_enter(move |_| *slot.borrow_mut() = Some(k.clone()));
        widget.add_controller(controller);
        self.keyed
            .borrow_mut()
            .push((key.to_string(), widget.clone().upcast()));
    }

    /// Filter button: icon or text, badge, name for the screen reader ("Caption: N unread").
    fn chip(
        self: &Rc<Self>,
        key: &str,
        glyph: gtk::Widget,
        label: &str,
        badge: &Badge,
        active: bool,
        msg: NotifMsg,
    ) -> gtk::Button {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&glyph));
        let badge_widget = badge_label(badge);
        badge_widget.set_halign(gtk::Align::End);
        badge_widget.set_valign(gtk::Align::Start);
        overlay.add_overlay(&badge_widget);
        let button = gtk::Button::new();
        button.set_child(Some(&overlay));
        button.add_css_class("shell-btn");
        button.add_css_class("notif-chip");
        if active {
            button.add_css_class("active");
        }
        let name = if badge.count > 0 {
            badge.a11y_name.as_str()
        } else {
            label
        };
        a11y::set_name(&button, name);
        button.set_tooltip_text(Some(label));
        let send = self.send.clone();
        button.connect_clicked(move |_| send(msg.clone()));
        self.track(key, &button);
        button
    }

    fn text_glyph(text: &str) -> gtk::Widget {
        let label = gtk::Label::new(Some(text));
        label.set_use_markup(false);
        label.upcast()
    }

    fn forget(&self, prefixes: &[&str]) {
        self.keyed
            .borrow_mut()
            .retain(|(k, _)| !prefixes.iter().any(|p| k.starts_with(p)));
    }

    fn build_filters(self: &Rc<Self>, all: &AllChip, kinds: &[KindChip]) {
        self.forget(&["all", "kind:", "sub:"]);
        clear(&self.filters);
        clear(&self.subs);
        self.filters.append(&self.chip(
            "all",
            Self::text_glyph(&all.label),
            &all.label,
            &all.badge,
            all.active,
            NotifMsg::SelectAll,
        ));
        for k in kinds {
            let glyph = icon(Some(k.icon)).upcast();
            self.filters.append(&self.chip(
                &format!("kind:{}", k.kind.as_str()),
                glyph,
                &k.label,
                &k.badge,
                k.active,
                NotifMsg::SelectKind(k.kind),
            ));
            for s in &k.subs {
                self.subs.append(&self.chip(
                    &format!("sub:{}.{}", k.kind.as_str(), s.sub),
                    icon(s.icon.as_deref()).upcast(),
                    &s.label,
                    &s.badge,
                    s.active,
                    NotifMsg::SelectSub(k.kind, s.sub.clone()),
                ));
            }
        }
        self.subs_row.set_visible(self.subs.first_child().is_some());
    }

    fn build_card(self: &Rc<Self>, c: &EventCard) -> gtk::Box {
        let card = a11y::landmark(
            gtk::AccessibleRole::Group,
            gtk::Orientation::Vertical,
            &format!("{}: {}", c.sub_label, c.text),
        );
        card.add_css_class("notif-card");
        card.add_css_class(card_class(c.accent));
        if c.unread {
            card.add_css_class("unread");
        }
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.append(&icon(c.icon.as_deref()));
        let sub = plain("notif-sub", &c.sub_label);
        sub.set_hexpand(true);
        head.append(&sub);
        if let Some(new) = &c.new_label {
            head.append(&plain("notif-new", new));
        }
        head.append(&plain("notif-time", &c.time));
        card.append(&head);
        if let Some(session) = &c.session_label {
            card.append(&plain("notif-session", session));
        }
        let text = plain("notif-text", &c.text);
        // the event text comes from outside and may be arbitrarily long: the card shows the first lines
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.set_lines(CARD_TEXT_LINES);
        card.append(&text);
        let button = gtk::Button::with_label(&c.action_label);
        button.add_css_class("shell-btn");
        button.set_halign(gtk::Align::End);
        let (send, id) = (self.send.clone(), c.id);
        button.connect_clicked(move |_| send(NotifMsg::Open(id)));
        self.track(&format!("card:{}", c.id), &button);
        card.append(&button);
        card
    }

    fn build_cards(self: &Rc<Self>, cards: &[EventCard]) {
        self.forget(&["card:"]);
        let vadj = self.scroller.vadjustment();
        let at = vadj.value();
        clear(&self.list);
        for c in cards.iter().take(MAX_CARDS) {
            self.list.append(&self.build_card(c));
        }
        gtk::glib::idle_add_local_once(move || vadj.set_value(at));
    }

    fn build_strip(self: &Rc<Self>, items: &[StripItem]) {
        self.forget(&["strip:"]);
        clear(&self.strip);
        for item in items {
            let button = self.chip(
                &format!("strip:{}", item.kind.as_str()),
                icon(Some(item.icon)).upcast(),
                &item.label,
                &item.badge,
                false,
                NotifMsg::StripClick(item.kind),
            );
            self.strip.append(&button);
        }
    }

    pub fn apply(self: &Rc<Self>, view: &NotifView) {
        self.expanded.set_visible(!view.collapsed);
        self.strip.set_visible(view.collapsed);
        let restore = self.focus_key.borrow().clone();
        if view.collapsed {
            if self.last_strip.borrow().as_ref() != Some(&view.strip) {
                self.build_strip(&view.strip);
                *self.last_strip.borrow_mut() = Some(view.strip.clone());
            }
        } else {
            let filters = (view.all.clone(), view.kinds.clone());
            if self.last_filters.borrow().as_ref() != Some(&filters) {
                self.build_filters(&filters.0, &filters.1);
                *self.last_filters.borrow_mut() = Some(filters);
            }
            if self.last_cards.borrow().as_ref() != Some(&view.cards) {
                self.build_cards(&view.cards);
                *self.last_cards.borrow_mut() = Some(view.cards.clone());
            }
            self.empty.set_text(view.empty.as_deref().unwrap_or(""));
            self.empty.set_visible(view.empty.is_some());
        }
        if let Some(key) = restore
            && let Some((_, w)) = self.keyed.borrow().iter().find(|(k, _)| *k == key)
        {
            w.grab_focus();
        }
    }

    /// Scrollable list (for UI scenarios).
    pub fn scroller(&self) -> gtk::ScrolledWindow {
        self.scroller.clone()
    }

    pub fn list(&self) -> gtk::Box {
        self.list.clone()
    }

    pub fn filters(&self) -> gtk::Box {
        self.filters.clone()
    }

    pub fn subs(&self) -> gtk::Box {
        self.subs.clone()
    }

    pub fn strip(&self) -> gtk::Box {
        self.strip.clone()
    }

    pub fn empty_label(&self) -> gtk::Label {
        self.empty.clone()
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
