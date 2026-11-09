//! Settings dialog: a layer over the window, a card up to 960 px wide and at most 90% of the window height, navigation by group
//! on the left, a scrolling page on the right, "Save" and "Close" below outside the scroll (always visible). What to show is
//! decided by `SettingsController`; this file has widgets only. Esc and a click on the backdrop go to the controller.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use controllers::i18n::tr;
use controllers::settings_types::Close;
use controllers::settings_view::{BannerKind, NavGroup, SettingsMsg, SettingsView};
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::a11y;
use crate::settings_rows::{Rows, Sender, plain};

/// Card width and its share of the window height.
const MAX_WIDTH: i32 = 960;
const MAX_HEIGHT_SHARE: f64 = 0.9;
/// Height of the card frame around the scrollable part: title, banner, confirmation, buttons, fields.
const CHROME: i32 = 230;

pub struct SettingsDialog {
    pub layer: gtk::Box,
    pub card: gtk::Box,
    pub save: gtk::Button,
    pub close: gtk::Button,
    pub rows: Rc<Rows>,
    title: gtk::Label,
    nav: gtk::Box,
    nav_scroll: gtk::ScrolledWindow,
    scroll: gtk::ScrolledWindow,
    banner: gtk::Label,
    confirm: gtk::Box,
    confirm_title: gtk::Label,
    confirm_lines: gtk::Box,
    yes: gtk::Button,
    no: gtk::Button,
    shown_nav: RefCell<Vec<NavGroup>>,
    was_open: Cell<bool>,
    send: Sender,
}

fn button(text: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("shell-btn");
    a11y::set_name(&b, text);
    b
}

impl SettingsDialog {
    pub fn new(send: Sender) -> Rc<SettingsDialog> {
        let layer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        layer.add_css_class("dialog-dim");
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_visible(false);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::Dialog)
            .spacing(8)
            .hexpand(true)
            .build();
        card.add_css_class("dialog-card");
        a11y::set_name(&card, &tr("set-title"));
        let title = plain("", "dialog-title");
        let nav = gtk::Box::new(gtk::Orientation::Vertical, 2);
        a11y::set_name(&nav, &tr("set-nav-name"));
        let nav_scroll = gtk::ScrolledWindow::new();
        nav_scroll.set_child(Some(&nav));
        nav_scroll.set_min_content_width(190);
        nav_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        nav_scroll.set_propagate_natural_height(true);
        let rows = Rows::new(send.clone());
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&rows.root));
        scroll.set_hexpand(true);
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_propagate_natural_height(true);
        a11y::set_name(&scroll, &tr("set-content-name"));
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        body.append(&nav_scroll);
        body.append(&scroll);
        let banner = plain("", "");
        banner.set_visible(false);
        let (confirm, confirm_title, confirm_lines, yes, no) = confirm_box();
        let save = button(&tr("set-save"));
        let close = button(&tr("set-close"));
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.set_halign(gtk::Align::End);
        footer.append(&close);
        footer.append(&save);
        card.append(&title);
        card.append(&body);
        card.append(&banner);
        card.append(&confirm);
        card.append(&footer);
        let clamp = adw::Clamp::builder()
            .maximum_size(MAX_WIDTH)
            .tightening_threshold(MAX_WIDTH)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .margin_start(16)
            .margin_end(16)
            .build();
        clamp.set_child(Some(&card));
        layer.append(&clamp);
        let dialog = Rc::new(SettingsDialog {
            layer,
            card,
            save,
            close,
            rows,
            title,
            nav,
            nav_scroll,
            scroll,
            banner,
            confirm,
            confirm_title,
            confirm_lines,
            yes,
            no,
            shown_nav: RefCell::new(Vec::new()),
            was_open: Cell::new(false),
            send,
        });
        dialog.fit(640);
        dialog.connect();
        dialog
    }

    fn connect(self: &Rc<Self>) {
        let send = self.send.clone();
        self.save.connect_clicked(move |_| send(SettingsMsg::Save));
        let send = self.send.clone();
        self.close
            .connect_clicked(move |_| send(SettingsMsg::Close(Close::Button)));
        let send = self.send.clone();
        self.yes
            .connect_clicked(move |_| send(SettingsMsg::Confirm(true)));
        let send = self.send.clone();
        self.no
            .connect_clicked(move |_| send(SettingsMsg::Confirm(false)));
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(d) = weak.upgrade() else { return };
            let hit = d.layer.pick(x, y, gtk::PickFlags::DEFAULT);
            if hit.is_some_and(|w| w == *d.layer.upcast_ref::<gtk::Widget>()) {
                (d.send)(SettingsMsg::Close(Close::Backdrop));
            }
        });
        self.layer.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let send = self.send.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                send(SettingsMsg::Close(Close::Esc));
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        self.layer.add_controller(keys);
    }

    /// Window height changed: the card is at most 90% of the window and only the page scrolls.
    pub fn fit(&self, window_height: i32) {
        let card = (f64::from(window_height) * MAX_HEIGHT_SHARE) as i32;
        let inner = (card - CHROME).max(120);
        self.scroll.set_max_content_height(inner);
        self.nav_scroll.set_max_content_height(inner);
    }

    fn nav(&self, groups: &[NavGroup]) {
        if *self.shown_nav.borrow() == groups {
            return;
        }
        *self.shown_nav.borrow_mut() = groups.to_vec();
        while let Some(c) = self.nav.first_child() {
            self.nav.remove(&c);
        }
        for g in groups {
            self.nav.append(&plain(&g.title, "caption"));
            for item in &g.items {
                let b = gtk::Button::with_label(&item.label);
                b.add_css_class("set-nav-btn");
                if item.selected {
                    b.add_css_class("selected");
                }
                a11y::set_name(&b, &item.label);
                let (send, page) = (self.send.clone(), item.page);
                b.connect_clicked(move |_| send(SettingsMsg::Select(page)));
                self.nav.append(&b);
            }
        }
    }

    /// Show the view: layer, navigation, page, banner, confirmation, button state.
    pub fn apply(&self, v: &SettingsView) {
        if !v.open {
            self.layer.set_visible(false);
            self.was_open.set(false);
            return;
        }
        self.title.set_text(&v.title);
        self.nav(&v.nav);
        self.rows.apply(&v.rows);
        match &v.banner {
            Some(b) => {
                self.banner.set_text(&crate::settings_rows::shown(&b.text));
                self.banner.remove_css_class("dialog-error");
                if b.kind == BannerKind::Error {
                    self.banner.add_css_class("dialog-error");
                }
                self.banner.set_visible(true);
            }
            None => self.banner.set_visible(false),
        }
        match &v.confirm {
            Some(c) => {
                self.confirm_title.set_text(&c.title);
                while let Some(ch) = self.confirm_lines.first_child() {
                    self.confirm_lines.remove(&ch);
                }
                for line in &c.lines {
                    self.confirm_lines.append(&plain(line, ""));
                }
                self.confirm.set_visible(true);
            }
            None => self.confirm.set_visible(false),
        }
        self.save.set_sensitive(v.dirty && !v.saving);
        self.layer.set_visible(true);
        if !self.was_open.replace(true) {
            self.close.grab_focus();
        }
    }

    /// Button by caption among the card's buttons (for UI scenarios).
    pub fn find_button(&self, label: &str) -> Option<gtk::Button> {
        fn search(w: &gtk::Widget, label: &str) -> Option<gtk::Button> {
            if let Ok(b) = w.clone().downcast::<gtk::Button>()
                && b.label().is_some_and(|l| l == label)
            {
                return Some(b);
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                if let Some(f) = search(&x, label) {
                    return Some(f);
                }
                c = x.next_sibling();
            }
            None
        }
        search(self.card.upcast_ref(), label)
    }

    pub fn banner_text(&self) -> Option<String> {
        self.banner
            .is_visible()
            .then(|| self.banner.text().to_string())
    }

    pub fn confirm_visible(&self) -> bool {
        self.confirm.is_visible()
    }

    /// Height of the scrollable part and of the card (for geometry UI scenarios).
    pub fn scroll_widget(&self) -> gtk::ScrolledWindow {
        self.scroll.clone()
    }
}

fn confirm_box() -> (gtk::Box, gtk::Label, gtk::Box, gtk::Button, gtk::Button) {
    let confirm = gtk::Box::new(gtk::Orientation::Vertical, 4);
    confirm.add_css_class("set-item");
    confirm.set_visible(false);
    a11y::set_name(&confirm, &tr("ss-confirm-name"));
    let title = plain("", "heading");
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let yes = button(&tr("ss-confirm-yes"));
    let no = button(&tr("ss-confirm-no"));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.append(&yes);
    bar.append(&no);
    confirm.append(&title);
    confirm.append(&lines);
    confirm.append(&bar);
    (confirm, title, lines, yes, no)
}
