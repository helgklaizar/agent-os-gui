//! Settings page rows from the `SettingsView`: fields, entries, buttons. Widgets are persistent until the structure
//! "signature" changes (set of rows, captions, choice lists): user input does not change the structure, so
//! focus and cursor are not lost. All external text is shown as is (`use_markup = false`), long text is truncated.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use controllers::settings_view::{ItemRow, Row, RowAction, SettingsMsg};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

pub type Sender = Rc<dyn Fn(SettingsMsg)>;

/// How many characters of external text go into a caption (the full text stays in the data).
const SHOWN: usize = 400;

pub fn shown(text: &str) -> String {
    if text.chars().count() <= SHOWN {
        return text.to_string();
    }
    let mut out: String = text.chars().take(SHOWN).collect();
    out.push('…');
    out
}

pub fn plain(text: &str, css: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&shown(text)));
    label.set_use_markup(false);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_selectable(false);
    if !css.is_empty() {
        label.add_css_class(css);
    }
    label
}

pub(crate) enum Control {
    Entry(gtk::Entry),
    Secret(gtk::PasswordEntry),
    Switch(gtk::Switch),
    Drop(gtk::DropDown, Vec<String>),
    Area(gtk::TextBuffer),
    Read(gtk::Label),
}

pub(crate) struct FieldWidgets {
    pub(crate) control: Control,
    pub(crate) err: gtk::Label,
    pub(crate) row: gtk::Box,
}

pub struct Rows {
    pub root: gtk::Box,
    pub(crate) fields: RefCell<HashMap<String, FieldWidgets>>,
    signature: RefCell<Vec<String>>,
    pub(crate) flag: Rc<Cell<bool>>,
    pub(crate) send: Sender,
}

fn action_button(a: &RowAction, send: &Sender) -> gtk::Button {
    let b = gtk::Button::with_label(&shown(&a.label));
    b.add_css_class("shell-btn");
    if a.destructive {
        b.add_css_class("destructive-action");
    }
    a11y::set_name(&b, &a.label);
    let (send, action) = (send.clone(), a.action.clone());
    b.connect_clicked(move |_| send(SettingsMsg::Act(action.clone())));
    b
}

fn signature(row: &Row) -> String {
    match row {
        Row::Heading(t) => format!("H:{t}"),
        Row::Note(t) => format!("N:{t}"),
        Row::Field(f) => format!("F:{}:{}:{:?}", f.id, f.label, f.kind),
        Row::Item(i) => format!(
            "I:{}|{}|{:?}|{:?}|{}",
            i.title,
            i.subtitle,
            i.tags,
            i.actions.iter().map(|a| &a.label).collect::<Vec<_>>(),
            i.selected
        ),
        Row::Button(a) => format!("B:{}", a.label),
    }
}

impl Rows {
    pub fn new(send: Sender) -> Rc<Rows> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_margin_top(4);
        root.set_margin_bottom(8);
        Rc::new(Rows {
            root,
            fields: RefCell::new(HashMap::new()),
            signature: RefCell::new(Vec::new()),
            flag: Rc::new(Cell::new(false)),
            send,
        })
    }

    fn item(&self, i: &ItemRow) -> gtk::Box {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("set-item");
        if i.selected {
            card.add_css_class("selected");
        }
        card.append(&plain(&i.title, "heading"));
        if !i.subtitle.is_empty() {
            card.append(&plain(&i.subtitle, "muted"));
        }
        for tag in &i.tags {
            card.append(&plain(tag, "muted"));
        }
        if !i.actions.is_empty() {
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for a in &i.actions {
                bar.append(&action_button(a, &self.send));
            }
            card.append(&bar);
        }
        card
    }

    /// Update values, errors and marks without recreating widgets.
    fn refresh(&self, rows: &[Row]) {
        self.flag.set(true);
        let fields = self.fields.borrow();
        for r in rows {
            let Row::Field(f) = r else { continue };
            let Some(w) = fields.get(&f.id) else { continue };
            match &f.error {
                Some(e) => {
                    w.err.set_text(&shown(e));
                    w.err.set_visible(true);
                }
                None => w.err.set_visible(false),
            }
            if f.dirty {
                w.row.add_css_class("dirty");
            } else {
                w.row.remove_css_class("dirty");
            }
            match &w.control {
                Control::Entry(e) if e.text() != f.value => e.set_text(&f.value),
                Control::Secret(e) if !f.dirty && !e.text().is_empty() => e.set_text(""),
                Control::Switch(s) if s.is_active() != (f.value == "true") => {
                    s.set_active(f.value == "true");
                }
                Control::Drop(d, values) => {
                    if let Some(i) = values.iter().position(|v| *v == f.value)
                        && d.selected() as usize != i
                    {
                        d.set_selected(i as u32);
                    }
                }
                Control::Area(b) => {
                    let now = b.text(&b.start_iter(), &b.end_iter(), false);
                    if now.as_str() != f.value {
                        b.set_text(&f.value);
                    }
                }
                Control::Read(l) => l.set_text(&shown(&f.value)),
                _ => {}
            }
        }
        self.flag.set(false);
    }

    pub fn apply(&self, rows: &[Row]) {
        let sig: Vec<String> = rows.iter().map(signature).collect();
        if *self.signature.borrow() == sig {
            self.refresh(rows);
            return;
        }
        *self.signature.borrow_mut() = sig;
        self.fields.borrow_mut().clear();
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        for r in rows {
            let widget: gtk::Widget = match r {
                Row::Heading(t) => {
                    let l = plain(t, "heading");
                    l.set_margin_top(10);
                    l.upcast()
                }
                Row::Note(t) => plain(t, "muted").upcast(),
                Row::Field(f) => self.field(f).upcast(),
                Row::Item(i) => self.item(i).upcast(),
                Row::Button(a) => {
                    let b = action_button(a, &self.send);
                    b.set_halign(gtk::Align::Start);
                    b.upcast()
                }
            };
            self.root.append(&widget);
        }
        self.refresh(rows);
    }

    /// Field widget by identifier (for UI scenarios): an entry or a switch.
    pub fn control(&self, id: &str) -> Option<gtk::Widget> {
        self.fields.borrow().get(id).map(|w| match &w.control {
            Control::Entry(e) => e.clone().upcast(),
            Control::Secret(e) => e.clone().upcast(),
            Control::Switch(s) => s.clone().upcast(),
            Control::Drop(d, _) => d.clone().upcast(),
            Control::Read(l) => l.clone().upcast(),
            Control::Area(_) => w.row.clone().upcast(),
        })
    }

    /// All page captions (for UI scenarios).
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
        walk(self.root.upcast_ref(), &mut out);
        out
    }
}
