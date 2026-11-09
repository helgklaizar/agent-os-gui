//! Building a Settings field: caption, input element by field kind, error line. Widget signals send messages
//! to the controller unless the view itself is switching values (`flag`).

use controllers::settings_types::Secret;
use controllers::settings_view::{FieldRow, Kind, SettingsMsg};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::settings_rows::{Control, FieldWidgets, Rows, plain, shown};

impl Rows {
    pub(crate) fn field(&self, f: &FieldRow) -> gtk::Box {
        let (send, flag, id) = (self.send.clone(), self.flag.clone(), f.id.clone());
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = plain(&f.label, "caption");
        let control = match &f.kind {
            Kind::Text | Kind::Number => {
                let e = gtk::Entry::new();
                e.set_hexpand(true);
                a11y::set_name(&e, &f.label);
                e.connect_changed(move |e| {
                    if !flag.get() {
                        send(SettingsMsg::Edit {
                            id: id.clone(),
                            value: e.text().to_string(),
                        });
                    }
                });
                Control::Entry(e)
            }
            Kind::Secret => {
                let e = gtk::PasswordEntry::new();
                e.set_hexpand(true);
                e.set_show_peek_icon(false);
                a11y::set_name(&e, &f.label);
                e.connect_changed(move |e| {
                    if !flag.get() {
                        send(SettingsMsg::EditSecret {
                            id: id.clone(),
                            value: Secret::new(e.text().to_string()),
                        });
                    }
                });
                Control::Secret(e)
            }
            Kind::Toggle => {
                let s = gtk::Switch::new();
                s.set_halign(gtk::Align::Start);
                a11y::set_name(&s, &f.label);
                s.connect_active_notify(move |s| {
                    if !flag.get() {
                        send(SettingsMsg::Edit {
                            id: id.clone(),
                            value: s.is_active().to_string(),
                        });
                    }
                });
                Control::Switch(s)
            }
            Kind::Choice(options) => {
                let labels: Vec<String> = options.iter().map(|(_, l)| shown(l)).collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                let d = gtk::DropDown::from_strings(&refs);
                a11y::set_name(&d, &f.label);
                let values: Vec<String> = options.iter().map(|(v, _)| v.clone()).collect();
                let vals = values.clone();
                d.connect_selected_notify(move |d| {
                    if flag.get() {
                        return;
                    }
                    if let Some(v) = vals.get(d.selected() as usize) {
                        send(SettingsMsg::Edit {
                            id: id.clone(),
                            value: v.clone(),
                        });
                    }
                });
                Control::Drop(d, values)
            }
            Kind::TextArea => {
                let view = gtk::TextView::new();
                view.set_wrap_mode(gtk::WrapMode::WordChar);
                a11y::set_name(&view, &f.label);
                let buffer = view.buffer();
                buffer.connect_changed(move |b| {
                    if !flag.get() {
                        let text = b.text(&b.start_iter(), &b.end_iter(), false);
                        send(SettingsMsg::Edit {
                            id: id.clone(),
                            value: text.to_string(),
                        });
                    }
                });
                let scroll = gtk::ScrolledWindow::new();
                scroll.set_min_content_height(160);
                scroll.set_child(Some(&view));
                row.append(&title);
                row.append(&scroll);
                self.finish(f, row.clone(), Control::Area(buffer));
                return row;
            }
            Kind::ReadOnly => Control::Read(plain(&f.value, "")),
        };
        row.append(&title);
        match &control {
            Control::Entry(w) => row.append(w),
            Control::Secret(w) => row.append(w),
            Control::Switch(w) => row.append(w),
            Control::Drop(w, _) => row.append(w),
            Control::Read(w) => row.append(w),
            Control::Area(_) => {}
        }
        self.finish(f, row.clone(), control);
        row
    }

    pub(crate) fn finish(&self, f: &FieldRow, row: gtk::Box, control: Control) {
        let err = plain("", "dialog-error");
        err.set_visible(false);
        row.append(&err);
        self.fields
            .borrow_mut()
            .insert(f.id.clone(), FieldWidgets { control, err, row });
    }
}
