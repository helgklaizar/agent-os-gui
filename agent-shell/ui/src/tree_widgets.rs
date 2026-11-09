//! Widgets of the "Sessions" column: tree rows, the "New session" and "New folder" buttons, the collapsed-column strip.
//! No logic here: rows, badges, colours and names arrive ready-made from `TreeView`; user text is shown
//! as plain text (no Pango markup), a persona colour is validated as `#rrggbb`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::i18n::tr;
use controllers::tree_types::NodeId;
use controllers::tree_view::{Strip, TreeView};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::tree_row::{RowWidgets, Sender, build_row, plain, update_row};

pub struct TreeWidgets {
    /// Column content: a list with buttons or the collapsed-column strip.
    pub root: gtk::Box,
    list_box: gtk::Box,
    scroller: gtk::ScrolledWindow,
    expanded: gtk::Box,
    strip: gtk::Box,
    pub new_session: gtk::Button,
    pub new_folder: gtk::Button,
    rows: RefCell<Vec<RowWidgets>>,
    /// Do not send `Focus` while code moves the focus.
    quiet: Rc<Cell<bool>>,
    send: Sender,
}

impl TreeWidgets {
    pub fn new(send: Sender) -> Rc<TreeWidgets> {
        let list_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::Tree)
            .build();
        list_box.add_css_class("tree-list");
        a11y::set_name(&list_box, &tr("column-sessions"));
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list_box)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let new_session = gtk::Button::new();
        let new_folder = gtk::Button::new();
        let buttons = gtk::Box::new(gtk::Orientation::Vertical, 4);
        buttons.add_css_class("tree-buttons");
        for (button, msg) in [
            (&new_session, TreeRequest::NewSession { folder: None }),
            (&new_folder, TreeRequest::NewFolder { parent: None }),
        ] {
            button.add_css_class("shell-btn");
            let send = send.clone();
            button.connect_clicked(move |_| send(TreeMsg::Request(msg)));
            buttons.append(button);
        }
        let expanded = gtk::Box::new(gtk::Orientation::Vertical, 0);
        expanded.set_vexpand(true);
        expanded.append(&scroller);
        expanded.append(&buttons);
        let strip = gtk::Box::new(gtk::Orientation::Vertical, 6);
        strip.add_css_class("tree-strip");
        strip.set_halign(gtk::Align::Center);
        strip.set_visible(false);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        root.append(&expanded);
        root.append(&strip);
        Rc::new(TreeWidgets {
            root,
            list_box,
            scroller,
            expanded,
            strip,
            new_session,
            new_folder,
            rows: RefCell::new(Vec::new()),
            quiet: Rc::new(Cell::new(false)),
            send,
        })
    }

    /// Show the list or the collapsed-column strip.
    pub fn apply(self: &Rc<Self>, view: &TreeView) {
        self.expanded.set_visible(!view.collapsed);
        self.strip.set_visible(view.collapsed);
        self.new_session.set_label(&view.new_session);
        self.new_folder.set_label(&view.new_folder);
        a11y::set_name(&self.new_session, &view.new_session);
        a11y::set_name(&self.new_folder, &view.new_folder);
        match &view.strip {
            Some(strip) => self.apply_strip(strip),
            None => self.apply_rows(view),
        }
    }

    fn apply_rows(self: &Rc<Self>, view: &TreeView) {
        let same = {
            let rows = self.rows.borrow();
            rows.len() == view.rows.len()
                && rows.iter().zip(&view.rows).all(|(w, r)| w.node == r.node)
        };
        self.quiet.set(true);
        if !same {
            while let Some(child) = self.list_box.first_child() {
                self.list_box.remove(&child);
            }
            let built: Vec<RowWidgets> = view
                .rows
                .iter()
                .map(|r| build_row(&self.send, &self.quiet, r))
                .collect();
            for w in &built {
                self.list_box.append(&w.root);
            }
            *self.rows.borrow_mut() = built;
        }
        let first = view.rows.first().map(|r| r.node);
        for (w, r) in self.rows.borrow().iter().zip(&view.rows) {
            update_row(w, r);
            // a single Tab entry point: the focused row (or the first one)
            w.root.set_focusable(view.focus.or(first) == Some(w.node));
        }
        self.quiet.set(false);
    }

    fn apply_strip(self: &Rc<Self>, strip: &Strip) {
        while let Some(child) = self.strip.first_child() {
            self.strip.remove(&child);
        }
        self.quiet.set(true);
        a11y::set_name(&self.strip, &strip.a11y_name);
        for item in &strip.items {
            let overlay = gtk::Overlay::new();
            let button = gtk::Button::with_label(&item.initial);
            button.add_css_class("shell-btn");
            a11y::set_name(&button, &item.label);
            button.set_tooltip_text(Some(&item.label));
            let (send, id) = (self.send.clone(), item.session_id);
            button.connect_clicked(move |_| send(TreeMsg::Select(id)));
            overlay.set_child(Some(&button));
            let badge = plain("tree-badge");
            badge.set_text(&item.badge);
            badge.set_halign(gtk::Align::End);
            badge.set_valign(gtk::Align::Start);
            overlay.add_overlay(&badge);
            self.strip.append(&overlay);
        }
        self.quiet.set(false);
    }

    /// Give keyboard focus to a row (after arrow keys and after a dialog closes).
    pub fn focus_node(&self, node: Option<NodeId>) {
        let Some(node) = node else { return };
        self.quiet.set(true);
        if let Some(w) = self.rows.borrow().iter().find(|w| w.node == node) {
            w.root.set_focusable(true);
            w.root.grab_focus();
        }
        self.quiet.set(false);
    }

    /// Tree row by node (for UI scenarios).
    pub fn row_widget(&self, node: NodeId) -> Option<gtk::Box> {
        self.rows
            .borrow()
            .iter()
            .find(|w| w.node == node)
            .map(|w| w.root.clone())
    }

    /// Scrollable list area (for UI scenarios).
    pub fn scroller(&self) -> gtk::ScrolledWindow {
        self.scroller.clone()
    }

    pub fn rows_len(&self) -> usize {
        self.rows.borrow().len()
    }
}
