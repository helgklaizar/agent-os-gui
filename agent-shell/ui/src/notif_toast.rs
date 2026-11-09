//! Toast: a transient "session: text" caption for 4 s. `ShellController` (`Toast`) decides timing and display; this file is the widget only.

use std::rc::Rc;

use controllers::controllers::shell::Toast;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

pub struct ToastView {
    pub root: gtk::Box,
    button: gtk::Button,
    label: gtk::Label,
    /// Session of the shown toast: clicking it opens that session.
    session: std::cell::Cell<Option<i64>>,
}

impl ToastView {
    /// `on_click` receives the id of the shown toast's session.
    pub fn new(on_click: Rc<dyn Fn(i64)>) -> Rc<ToastView> {
        let label = gtk::Label::new(None);
        label.set_use_markup(false);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(48);
        label.set_xalign(0.0);
        let button = gtk::Button::new();
        button.set_child(Some(&label));
        button.add_css_class("notif-toast");
        let root = a11y::landmark(
            gtk::AccessibleRole::Alert,
            gtk::Orientation::Vertical,
            "toast",
        );
        root.set_halign(gtk::Align::End);
        root.set_valign(gtk::Align::End);
        root.set_visible(false);
        root.append(&button);
        let view = Rc::new(ToastView {
            root,
            button,
            label,
            session: std::cell::Cell::new(None),
        });
        let weak = Rc::downgrade(&view);
        view.button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade()
                && let Some(id) = view.session.get()
            {
                on_click(id);
            }
        });
        view
    }

    pub fn apply(&self, toast: Option<&Toast>) {
        match toast {
            Some(t) => {
                self.label.set_text(&t.text);
                a11y::set_name(&self.root, &t.text);
                self.session.set(Some(t.session_id));
                self.root.set_visible(true);
            }
            None => {
                self.session.set(None);
                self.root.set_visible(false);
            }
        }
    }
}
