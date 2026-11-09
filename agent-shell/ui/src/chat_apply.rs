//! Applying `ChatView` to the chat widgets.

use std::rc::Rc;

use controllers::chat_view::ChatView;
use controllers::controllers::chat::ChatMsg;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::chat_items::item_widget;
use crate::chat_widgets::{ChatWidgets, WINDOW, flat};
use controllers::i18n::tr;

impl ChatWidgets {
    /// Apply the controller view to the widgets.
    pub fn apply(self: &Rc<Self>, v: &ChatView) {
        self.empty.set_visible(v.empty.is_some());
        self.main.set_visible(v.empty.is_none());
        if let Some(text) = &v.empty {
            self.empty.set_label(text);
            return;
        }
        self.apply_chips(v);
        self.apply_items(v);
        self.apply_input(v);
        match &v.typing {
            Some(t) => {
                self.typing.set_label(t);
                self.typing.set_visible(true);
            }
            None => self.typing.set_visible(false),
        }
        self.stop_btn.set_visible(v.can_stop);
        self.send_btn.set_sensitive(v.can_send);
    }

    fn apply_chips(&self, v: &ChatView) {
        self.chips.set_visible(v.chips.is_some());
        if let Some(c) = &v.chips {
            // the button label overrides the accessible name, so the chip's purpose is part of the visible text
            let text = |key: &str, label: &str| format!("{}: {label}", tr(key));
            let persona = text("chat-chip-persona-name", &c.persona_label);
            let path = text("chat-chip-path-name", &c.path_label);
            self.persona_chip.set_label(&persona);
            self.path_chip.set_label(&path);
            a11y::set_name(&self.persona_chip, &persona);
            a11y::set_name(&self.path_chip, &path);
        }
        while let Some(child) = self.persona_list.first_child() {
            self.persona_list.remove(&child);
        }
        self.persona_list.set_visible(v.persona_choices.is_some());
        for choice in v.persona_choices.iter().flatten() {
            let label = if choice.current {
                format!("✓ {}", choice.name)
            } else {
                choice.name.clone()
            };
            let b = flat(&label, "chat-persona-item");
            let (send, id) = (self.send.clone(), choice.id);
            b.connect_clicked(move |_| send(ChatMsg::PickPersona(id)));
            self.persona_list.append(&b);
        }
    }

    fn apply_input(&self, v: &ChatView) {
        let buf = self.input.buffer();
        let now = buf.text(&buf.start_iter(), &buf.end_iter(), false);
        if now.as_str() != v.draft {
            self.quiet.set(true);
            buf.set_text(&v.draft);
            self.quiet.set(false);
        }
        while let Some(child) = self.attachments.first_child() {
            self.attachments.remove(&child);
        }
        for (i, a) in v.attachments.iter().enumerate() {
            let b = flat(
                &format!("📎 {} ({}) ✕", a.name, a.size_text),
                "chat-attach-chip",
            );
            a11y::set_name(&b, &format!("{}: {}", tr("chat-attach-remove"), a.name));
            let send = self.send.clone();
            b.connect_clicked(move |_| send(ChatMsg::AttachRemove(i)));
            self.attachments.append(&b);
        }
    }

    fn apply_items(self: &Rc<Self>, v: &ChatView) {
        let mut s = self.shown.borrow_mut();
        if s.session != v.session_id || s.start == usize::MAX {
            for (_, w) in s.widgets.drain(..) {
                self.items.remove(&w);
            }
            if s.session != v.session_id {
                self.window.set(WINDOW);
            }
            s.session = v.session_id;
            s.start = v.items.len().saturating_sub(self.window.get());
        }
        let start = s.start;
        let slice = &v.items[start.min(v.items.len())..];
        self.earlier.set_visible(start > 0);
        s.widgets.truncate(slice.len());
        for (i, item) in slice.iter().enumerate() {
            if s.widgets.get(i).is_some_and(|(old, _)| old == item) {
                continue;
            }
            let w = item_widget(item, &self.send);
            if i < s.widgets.len() {
                let old = std::mem::replace(&mut s.widgets[i], (item.clone(), w.clone())).1;
                self.items.insert_child_after(&w, Some(&old));
                self.items.remove(&old);
            } else {
                self.items.append(&w);
                s.widgets.push((item.clone(), w));
            }
        }
        if v.scroll_seq != s.scroll_seq {
            s.scroll_seq = v.scroll_seq;
            let adj = self.scroller.vadjustment();
            gtk::glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
        }
    }
}
