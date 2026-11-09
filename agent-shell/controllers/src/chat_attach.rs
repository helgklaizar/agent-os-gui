//! Chat attachments: file selection, preview (name and size), list from `meta.attachments`. The file content is neither read nor shown.

use serde_json::Value;

use crate::chat_types::AttachmentRef;
use crate::chat_view::AttachPreview;
use crate::controller::Effect;
use crate::controllers::chat::ChatController;
use crate::i18n::{tr, tr_args};

/// Name without control characters and text-direction spoofing (RTL override): `evil\u{202E}fdp.exe` does not look like `evilexe.pdf`.
pub fn clean_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    if name.trim().is_empty() {
        tr("chat-attach-unnamed")
    } else {
        name
    }
}

fn tenths(n: u64, unit: u64) -> String {
    let t = n.saturating_mul(10) / unit;
    format!("{}.{}", t / 10, t % 10)
}

/// "512 B", "1.5 KB", "5.0 MB".
pub fn size_text(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    match size {
        s if s < KB => tr_args("chat-size-b", &[("n", &s.to_string())]),
        s if s < MB => tr_args("chat-size-kb", &[("n", &tenths(s, KB))]),
        s if s < GB => tr_args("chat-size-mb", &[("n", &tenths(s, MB))]),
        s => tr_args("chat-size-gb", &[("n", &tenths(s, GB))]),
    }
}

pub fn preview(name: &str, size: u64) -> AttachPreview {
    AttachPreview {
        name: clean_name(name),
        size_text: size_text(size),
    }
}

/// Message attachments from `meta.attachments` (`[{id, name, size, mime}]`).
pub fn message_previews(meta: &Value) -> Vec<AttachPreview> {
    meta.get("attachments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| {
                    let name = x.get("name").and_then(Value::as_str).unwrap_or_default();
                    preview(name, x.get("size").and_then(Value::as_u64).unwrap_or(0))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl ChatController {
    pub(crate) fn staged(&self) -> &[AttachmentRef] {
        self.session
            .and_then(|s| self.staged.get(&s))
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn staged_previews(&self) -> Vec<AttachPreview> {
        self.staged()
            .iter()
            .map(|a| preview(&a.name, a.size))
            .collect()
    }

    pub(crate) fn attach_picked(&mut self, path: String, name: String, size: u64) -> Vec<Effect> {
        self.want_attach_dialog = false;
        if let Some(s) = self.session {
            let list = self.staged.entry(s).or_default();
            if !list.iter().any(|a| a.path == path) {
                list.push(AttachmentRef {
                    path,
                    name: clean_name(&name),
                    size,
                });
            }
        }
        Vec::new()
    }

    pub(crate) fn attach_remove(&mut self, index: usize) {
        if let Some(list) = self.session.and_then(|s| self.staged.get_mut(&s))
            && index < list.len()
        {
            list.remove(index);
        }
    }
}
