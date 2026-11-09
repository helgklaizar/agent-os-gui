//! "Session permissions" block: one field each for `send`, `delete`, `shell`. Values that weaken a global or persona rule
//! are unavailable (the button is disabled and marked unavailable in the accessibility tree); under a higher prohibition the whole field is disabled.

use std::cell::Cell;
use std::rc::Rc;

use controllers::i18n::tr;
use controllers::permissions::Mode;
use controllers::ss_view::{PERM_ACTIONS, PermField, SsMsg};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

struct Row {
    action: &'static str,
    title: gtk::Label,
    inherit: gtk::ToggleButton,
    modes: Vec<(Mode, gtk::ToggleButton)>,
    note: gtk::Label,
}

pub struct PermsBlock {
    pub root: gtk::Box,
    rows: Vec<Row>,
    /// The view itself switches the buttons: handlers stay silent meanwhile.
    flag: Rc<Cell<bool>>,
}

impl PermsBlock {
    pub fn new(send: Rc<dyn Fn(SsMsg)>) -> Rc<PermsBlock> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let heading = gtk::Label::new(Some(&tr("ss-section-perms")));
        heading.set_use_markup(false);
        heading.set_xalign(0.0);
        heading.add_css_class("heading");
        root.append(&heading);
        let mut rows = Vec::new();
        let flag = Rc::new(Cell::new(false));
        for action in PERM_ACTIONS {
            let title = gtk::Label::new(None);
            title.set_use_markup(false);
            title.set_xalign(0.0);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            line.add_css_class("linked");
            let inherit = gtk::ToggleButton::with_label(&tr("ss-perm-inherit"));
            line.append(&inherit);
            let mut modes = Vec::new();
            for (mode, key) in [
                (Mode::Allow, "chat-mode-allow"),
                (Mode::Ask, "chat-mode-ask"),
                (Mode::Deny, "chat-mode-deny"),
            ] {
                let b = gtk::ToggleButton::with_label(&tr(key));
                b.set_group(Some(&inherit));
                line.append(&b);
                modes.push((mode, b));
            }
            let note = gtk::Label::new(None);
            note.set_use_markup(false);
            note.set_xalign(0.0);
            note.set_wrap(true);
            note.add_css_class("dim-label");
            root.append(&title);
            root.append(&line);
            root.append(&note);
            let s = send.clone();
            let echo = flag.clone();
            inherit.connect_toggled(move |b| {
                if b.is_active() && !echo.get() {
                    s(SsMsg::SetPerm {
                        action: action.to_string(),
                        mode: None,
                    });
                }
            });
            for (mode, b) in &modes {
                let (s, mode, flag) = (send.clone(), *mode, flag.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() && !flag.get() {
                        s(SsMsg::SetPerm {
                            action: action.to_string(),
                            mode: Some(mode),
                        });
                    }
                });
            }
            rows.push(Row {
                action,
                title,
                inherit,
                modes,
                note,
            });
        }
        Rc::new(PermsBlock { root, rows, flag })
    }

    pub fn apply(&self, fields: &[PermField]) {
        self.flag.set(true);
        {
            for f in fields {
                let Some(row) = self.rows.iter().find(|r| r.action == f.action) else {
                    continue;
                };
                row.title.set_text(&f.label);
                a11y::set_name(&row.title, &f.label);
                row.inherit.set_active(f.selected.is_none());
                row.inherit.set_sensitive(!f.locked);
                for (mode, b) in &row.modes {
                    let opt = f.options.iter().find(|o| o.mode == *mode);
                    b.set_active(f.selected == Some(*mode));
                    let enabled = opt.is_some_and(|o| o.enabled) && !f.locked;
                    b.set_sensitive(enabled);
                    let name = format!("{}: {}", f.label, opt.map_or("", |o| o.label.as_str()));
                    a11y::set_name(b, &name);
                    b.set_tooltip_text(if enabled { None } else { f.note.as_deref() });
                }
                row.note.set_text(f.note.as_deref().unwrap_or(""));
                row.note.set_visible(f.note.is_some());
            }
        }
        self.flag.set(false);
    }

    /// Permission value button (for UI scenarios).
    pub fn mode_button(&self, action: &str, mode: Mode) -> Option<gtk::ToggleButton> {
        self.rows
            .iter()
            .find(|r| r.action == action)?
            .modes
            .iter()
            .find(|(m, _)| *m == mode)
            .map(|(_, b)| b.clone())
    }
}
