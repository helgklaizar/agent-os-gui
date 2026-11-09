//! Tree dialog forms in `DialogHost`: name, folder list with indents, persona, delete
//! confirmation. `DialogController` decides what to show; widgets only draw `FormView` and send signals.

use std::cell::RefCell;

use controllers::DialogKind;
use controllers::controllers::dialog::{DialogMsg, FormView};
use controllers::i18n::tr;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::dialog_host::DialogHost;
use crate::theme_tokens::Rgb;

/// List item: id, caption, indent, dot colour, whether selected.
type RadioOption<'a> = (Option<i64>, &'a str, u32, Option<&'a str>, bool);

/// Live widgets of the open form: updated in place so that input does not lose focus on every letter.
pub struct FormUi {
    kind: DialogKind,
    /// Everything that requires a rebuild: title, captions and the composition of lists (without selected items and text).
    key: String,
    entry: Option<gtk::Entry>,
    error: gtk::Label,
    submit: gtk::Button,
    folders: Vec<(Option<i64>, gtk::CheckButton)>,
    personas: Vec<(Option<i64>, gtk::CheckButton)>,
}

fn structure_key(kind: DialogKind, f: &FormView) -> String {
    let folders: Vec<_> = f
        .folders
        .iter()
        .flatten()
        .map(|o| (o.id, &o.label, o.indent))
        .collect();
    let personas: Vec<_> = f
        .personas
        .iter()
        .flatten()
        .map(|o| (o.id, &o.label, &o.color))
        .collect();
    format!(
        "{kind:?}|{}|{}|{:?}|{folders:?}|{personas:?}|{:?}",
        f.title, f.submit_label, f.lines, f.has_name
    )
}

fn field_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_use_markup(false);
    label.set_xalign(0.0);
    label.add_css_class("dialog-key");
    label
}

fn plain_line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_use_markup(false);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("dialog-value");
    label
}

impl DialogHost {
    pub(crate) fn form_matches(&self, kind: DialogKind, form: &FormView) -> bool {
        self.form_ui
            .borrow()
            .as_ref()
            .is_some_and(|u| u.kind == kind && u.key == structure_key(kind, form))
    }

    /// Update the open form in place: hint, button, selected items, text.
    pub(crate) fn update_form(&self, form: &FormView) {
        let ui = self.form_ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        match &form.name_error {
            Some(text) => {
                ui.error.set_text(text);
                ui.error.set_visible(true);
            }
            None => ui.error.set_visible(false),
        }
        ui.submit.set_sensitive(form.can_submit);
        if let Some(entry) = &ui.entry
            && entry.text() != form.name
        {
            entry.set_text(&form.name);
        }
        let sync = |radios: &[(Option<i64>, gtk::CheckButton)], selected: Vec<Option<i64>>| {
            for (id, radio) in radios {
                radio.set_active(selected.contains(id));
            }
        };
        sync(
            &ui.folders,
            form.folders
                .iter()
                .flatten()
                .filter(|o| o.selected)
                .map(|o| o.id)
                .collect(),
        );
        sync(
            &ui.personas,
            form.personas
                .iter()
                .flatten()
                .filter(|o| o.selected)
                .map(|o| o.id)
                .collect(),
        );
    }

    pub(crate) fn rebuild_form(&self, kind: DialogKind, form: &FormView) {
        self.clear();
        a11y::set_name(&self.card, &form.title);
        let title = gtk::Label::new(Some(&form.title));
        title.set_use_markup(false);
        title.set_wrap(true);
        title.set_xalign(0.0);
        title.add_css_class("dialog-title");
        self.card.append(&title);
        for line in &form.lines {
            self.card.append(&plain_line(line));
        }
        let entry = form.has_name.then(|| self.name_field(form));
        let error = gtk::Label::new(None);
        error.set_use_markup(false);
        error.set_xalign(0.0);
        error.add_css_class("dialog-error");
        error.set_visible(false);
        self.card.append(&error);
        let folders = form.folders.as_ref().map(|list| {
            self.radio_group(
                &tr("dialog-label-folder"),
                list.iter()
                    .map(|o| (o.id, o.label.as_str(), o.indent, None, o.selected))
                    .collect(),
                DialogMsg::SetFolder,
            )
        });
        let personas = form.personas.as_ref().map(|list| {
            self.radio_group(
                &tr("dialog-label-persona"),
                list.iter()
                    .map(|o| (o.id, o.label.as_str(), 0, o.color.as_deref(), o.selected))
                    .collect(),
                DialogMsg::SetPersona,
            )
        });
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(12);
        let cancel = self.button(&tr("logout-cancel"), DialogMsg::Cancel);
        let submit = self.button(&form.submit_label, DialogMsg::Submit);
        submit.add_css_class(
            if matches!(
                kind,
                DialogKind::ConfirmDeleteSession | DialogKind::ConfirmDeleteFolder
            ) {
                "destructive-action"
            } else {
                "suggested-action"
            },
        );
        actions.append(&cancel);
        actions.append(&submit);
        self.card.append(&actions);
        let first_focus: gtk::Widget = match (&entry, &folders) {
            (Some(e), _) => e.clone().upcast(),
            (None, Some(list)) if !list.is_empty() => list[0].1.clone().upcast(),
            _ => cancel.clone().upcast(),
        };
        *self.form_ui.borrow_mut() = Some(FormUi {
            kind,
            key: structure_key(kind, form),
            entry,
            error,
            submit,
            folders: folders.unwrap_or_default(),
            personas: personas.unwrap_or_default(),
        });
        self.update_form(form);
        first_focus.grab_focus();
    }

    fn name_field(&self, form: &FormView) -> gtk::Entry {
        let label = tr("dialog-label-name");
        self.card.append(&field_label(&label));
        let entry = gtk::Entry::new();
        entry.set_text(&form.name);
        entry.set_activates_default(false);
        a11y::set_name(&entry, &label);
        let send = self.send.clone();
        entry.connect_changed(move |e| send(DialogMsg::SetName(e.text().to_string())));
        let send = self.send.clone();
        entry.connect_activate(move |_| send(DialogMsg::Submit));
        self.card.append(&entry);
        entry
    }

    /// A list of options with switches; indent by folder depth, for a persona a coloured dot in the caption.
    fn radio_group(
        &self,
        caption: &str,
        options: Vec<RadioOption<'_>>,
        msg: fn(Option<i64>) -> DialogMsg,
    ) -> Vec<(Option<i64>, gtk::CheckButton)> {
        self.card.append(&field_label(caption));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(60)
            .max_content_height(180)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        scroller.add_css_class("dialog-list");
        let radios = RefCell::new(Vec::new());
        let mut first: Option<gtk::CheckButton> = None;
        for (id, label, indent, color, selected) in options {
            let radio = gtk::CheckButton::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            if let Some(Rgb(r, g, b)) = color.and_then(Rgb::parse) {
                let dot = gtk::DrawingArea::new();
                dot.set_content_width(10);
                dot.set_content_height(10);
                dot.set_valign(gtk::Align::Center);
                dot.set_draw_func(move |_, cr, w, h| {
                    cr.set_source_rgb(
                        f64::from(r) / 255.0,
                        f64::from(g) / 255.0,
                        f64::from(b) / 255.0,
                    );
                    cr.arc(
                        f64::from(w) / 2.0,
                        f64::from(h) / 2.0,
                        f64::from(w.min(h)) / 2.0,
                        0.0,
                        std::f64::consts::TAU,
                    );
                    if let Err(e) = cr.fill() {
                        gtk::glib::g_warning!("agent-shell", "persona dot not drawn: {e}");
                    }
                });
                row.append(&dot);
            }
            let text = gtk::Label::new(Some(label));
            text.set_use_markup(false);
            text.set_xalign(0.0);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&text);
            radio.set_child(Some(&row));
            a11y::set_name(&radio, label);
            radio.set_margin_start(indent as i32);
            radio.set_active(selected);
            match &first {
                Some(head) => radio.set_group(Some(head)),
                None => first = Some(radio.clone()),
            }
            let send = self.send.clone();
            radio.connect_toggled(move |r| {
                if r.is_active() {
                    send(msg(id));
                }
            });
            list.append(&radio);
            radios.borrow_mut().push((id, radio));
        }
        self.card.append(&scroller);
        radios.into_inner()
    }
}
