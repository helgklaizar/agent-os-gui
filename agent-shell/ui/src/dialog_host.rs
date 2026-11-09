//! Modal dialog layer over the window: dimmed backdrop, About card or Logout confirmation.
//! `DialogController` decides what to show; this file has widgets only. Esc and a click on the backdrop go to the controller.

use std::cell::RefCell;
use std::rc::Rc;

use controllers::DialogKind;
use controllers::controllers::dialog::{DialogMsg, DialogView};
use controllers::i18n::tr;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::dialog_form_view::FormUi;

pub type Sender = Rc<dyn Fn(DialogMsg)>;

pub struct DialogHost {
    /// Layer over the window content (added to `gtk::Overlay`).
    pub layer: gtk::Box,
    pub(crate) card: gtk::Box,
    shown: RefCell<Option<(DialogKind, DialogView)>>,
    pub(crate) form_ui: RefCell<Option<FormUi>>,
    pub(crate) send: Sender,
}

fn plain(text: &str, css: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_use_markup(false);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_selectable(false);
    label.add_css_class(css);
    label
}

impl DialogHost {
    pub fn new(send: Sender) -> Rc<DialogHost> {
        let layer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        layer.add_css_class("dialog-dim");
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_visible(false);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::Dialog)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        card.add_css_class("dialog-card");
        card.set_size_request(360, -1);
        layer.append(&card);
        let host = Rc::new(DialogHost {
            layer,
            card,
            shown: RefCell::new(None),
            form_ui: RefCell::new(None),
            send,
        });
        host.connect_backdrop();
        host
    }

    /// A click on the backdrop (outside the card) closes the top dialog.
    fn connect_backdrop(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(host) = weak.upgrade() else { return };
            let hit = host.layer.pick(x, y, gtk::PickFlags::DEFAULT);
            if hit.is_some_and(|w| w == *host.layer.upcast_ref::<gtk::Widget>()) {
                (host.send)(DialogMsg::Backdrop);
            }
        });
        self.layer.add_controller(click);
    }

    pub fn is_open(&self) -> bool {
        self.layer.is_visible()
    }

    /// Show the top dialog of the stack or hide the layer. Focus is taken only when a new dialog opens.
    pub fn apply(&self, view: &DialogView) {
        let top = view.stack.last().copied();
        if let (Some(kind), Some(form)) = (top, &view.form) {
            if self.form_matches(kind, form) {
                self.update_form(form);
            } else {
                *self.shown.borrow_mut() = None;
                self.rebuild_form(kind, form);
            }
            self.layer.set_visible(true);
            return;
        }
        *self.form_ui.borrow_mut() = None;
        let (was_kind, same) = match self.shown.borrow().as_ref() {
            Some((k, v)) => (Some(*k), v == view),
            None => (None, false),
        };
        if same {
            return;
        }
        *self.shown.borrow_mut() = top.map(|k| (k, view.clone()));
        match top {
            None => self.layer.set_visible(false),
            Some(kind) => {
                self.rebuild(kind, view, was_kind != top);
                self.layer.set_visible(true);
            }
        }
    }

    pub(crate) fn clear(&self) {
        while let Some(child) = self.card.first_child() {
            self.card.remove(&child);
        }
    }

    fn rebuild(&self, kind: DialogKind, view: &DialogView, focus: bool) {
        self.clear();
        let title = match kind {
            DialogKind::About => tr("about-title"),
            DialogKind::ConfirmLogout => tr("logout-title"),
            _ => return,
        };
        a11y::set_name(&self.card, &title);
        self.card.append(&plain(&title, "dialog-title"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(12);
        let first_focus = match kind {
            DialogKind::About => {
                let a = &view.about;
                for (key, value) in [
                    (tr("about-label-version"), a.version.as_str()),
                    (tr("about-label-platform"), a.platform.as_str()),
                    (tr("about-label-config"), a.config_path.as_str()),
                ] {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let k = plain(&key, "dialog-key");
                    k.set_width_chars(14);
                    row.append(&k);
                    row.append(&plain(value, "dialog-value"));
                    self.card.append(&row);
                }
                let close = self.button(&tr("dialog-close"), DialogMsg::Cancel);
                actions.append(&close);
                close
            }
            DialogKind::ConfirmLogout => {
                self.card.append(&plain(&tr("logout-text"), "dialog-value"));
                let cancel = self.button(&tr("logout-cancel"), DialogMsg::Cancel);
                let confirm = self.button(&tr("logout-confirm"), DialogMsg::Confirm);
                confirm.add_css_class("destructive-action");
                actions.append(&cancel);
                actions.append(&confirm);
                cancel
            }
            _ => return,
        };
        self.card.append(&actions);
        if focus {
            first_focus.grab_focus();
        }
    }

    pub(crate) fn button(&self, label: &str, msg: DialogMsg) -> gtk::Button {
        let button = gtk::Button::with_label(label);
        button.add_css_class("shell-btn");
        let send = self.send.clone();
        button.connect_clicked(move |_| send(msg.clone()));
        button
    }

    /// Action button of the top dialog by caption (for UI scenarios).
    pub fn find_button(&self, label: &str) -> Option<gtk::Button> {
        let mut child = self.card.first_child();
        while let Some(w) = child {
            if let Some(found) = Self::search(&w, label) {
                return Some(found);
            }
            child = w.next_sibling();
        }
        None
    }

    fn search(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Ok(b) = widget.clone().downcast::<gtk::Button>()
            && b.label().is_some_and(|l| l == label)
        {
            return Some(b);
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            if let Some(found) = Self::search(&w, label) {
                return Some(found);
            }
            child = w.next_sibling();
        }
        None
    }

    /// All card captions: title and values (for UI scenarios).
    pub fn texts(&self) -> Vec<String> {
        fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
            if let Ok(l) = w.clone().downcast::<gtk::Label>() {
                out.push(l.text().to_string());
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                walk(&x, out);
                c = x.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(self.card.upcast_ref(), &mut out);
        out
    }
}
