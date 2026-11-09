//! Settings dialog state: core model, drafts, errors, confirmation. A draft is typed but unsaved
//! text; it lives until saved: switching pages, closing the dialog and a save error do not erase it.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::settings_entities::show_entity;
use crate::settings_fields::{Spec, Ty, show};
use crate::settings_model::CoreModel;
use crate::settings_types::{Action, Op, Secret};
use crate::settings_view::{Banner, FieldRow, Kind, Row};
use crate::types::{FocusTarget, SettingsPage, Theme};

/// An open editor file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub path: String,
    pub rev: String,
    /// Saved text (after `GET`); empty until there is a reply.
    pub text: String,
    pub loaded: bool,
}

/// An action awaiting confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConfirm {
    pub action: Action,
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Default)]
pub struct State {
    pub model: CoreModel,
    pub open: bool,
    pub page: Option<SettingsPage>,
    pub opener: Option<FocusTarget>,
    pub text: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, Secret>,
    pub errors: BTreeMap<String, String>,
    pub indicators: Option<Vec<Value>>,
    pub selected_persona: Option<i64>,
    pub perm_persona: Option<i64>,
    pub file: Option<OpenFile>,
    pub theme: Theme,
    pub system: String,
    pub config: String,
    pub banner: Option<Banner>,
    pub confirm: Option<PendingConfirm>,
    /// Commands that have no reply yet.
    pub pending: usize,
    /// What was sent to the core: field id -> value (after success only the unchanged drafts are erased).
    pub sent: BTreeMap<String, String>,
    /// At least one command of the current batch was rejected.
    pub failed: bool,
}

impl State {
    pub fn page(&self) -> SettingsPage {
        self.page.unwrap_or(SettingsPage::Account)
    }

    /// Saved field value: document or core record, text of the open file; a new record gets the default value.
    pub fn saved(&self, spec: &Spec) -> String {
        if spec.doc == "file" {
            return self
                .file
                .as_ref()
                .map(|f| f.text.clone())
                .unwrap_or_default();
        }
        if spec.doc.starts_with("persona:") || spec.doc.starts_with("module:") {
            return show_entity(spec, &self.model);
        }
        if spec.doc.ends_with("_new") {
            return spec.default.clone();
        }
        show(spec, &self.model)
    }

    /// Field value to show: draft or saved.
    pub fn value(&self, spec: &Spec) -> String {
        self.text
            .get(&spec.id)
            .cloned()
            .unwrap_or_else(|| self.saved(spec))
    }

    pub fn dirty(&self, spec: &Spec) -> bool {
        self.text.contains_key(&spec.id)
    }

    /// Record input: input equal to the saved value does not count as a draft.
    pub fn edit(&mut self, spec: &Spec, value: String) {
        self.errors.remove(&spec.id);
        if value == self.saved(spec) {
            self.text.remove(&spec.id);
        } else {
            self.text.insert(spec.id.clone(), value);
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.text.is_empty() || !self.secrets.is_empty() || self.indicators.is_some()
    }

    /// Ids of the drafts that belong to the operation.
    pub fn drafts_of(&self, op: &Op) -> Vec<String> {
        let own = |id: &str| match op {
            Op::Patch(doc) => {
                id.starts_with(&format!("d:{doc}:"))
                    || (doc == "connections" && id.starts_with("c:new:"))
            }
            Op::Persona => id.starts_with("p:"),
            Op::Account => id.starts_with("a:new:"),
            Op::Tunnel => id.starts_with("t:new:"),
            Op::Module => id.starts_with("m:"),
            Op::Permission => id.starts_with("perm:"),
            Op::File => id == "f:text",
            _ => false,
        };
        self.text.keys().filter(|k| own(k)).cloned().collect()
    }

    /// Erase what was saved: a draft whose value did not change after sending.
    pub fn clear_saved(&mut self, op: &Op) {
        for id in self.drafts_of(op) {
            if self.sent.get(&id) == self.text.get(&id) {
                self.text.remove(&id);
                self.sent.remove(&id);
            }
        }
    }
}

/// Field row: value, error, "changed" mark.
pub fn field_row(st: &State, spec: &Spec) -> Row {
    let kind = match &spec.ty {
        Ty::Text(max) if *max > 4096 => Kind::TextArea,
        Ty::Text(_) => Kind::Text,
        Ty::Toggle => Kind::Toggle,
        Ty::Choice(o) | Ty::ChoiceId(o) => Kind::Choice(o.clone()),
        Ty::Int(..) | Ty::Time => Kind::Number,
    };
    let value = st.value(spec);
    Row::Field(FieldRow {
        id: spec.id.clone(),
        label: spec.label.clone(),
        kind,
        value,
        error: st.errors.get(&spec.id).cloned(),
        dirty: st.dirty(spec),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fields::doc_field_id;
    use serde_json::json;

    fn spec() -> Spec {
        Spec {
            id: doc_field_id("account", "name"),
            doc: "account".into(),
            path: "name".into(),
            label: "x".into(),
            ty: Ty::Text(80),
            default: String::new(),
        }
    }

    #[test]
    fn edit_back_to_saved_value_clears_draft() {
        let mut st = State::default();
        st.model.docs.insert(
            "account".into(),
            crate::settings_model::Doc {
                value: json!({"name": "A"}),
                rev: 1,
            },
        );
        let s = spec();
        st.edit(&s, "B".into());
        assert!(st.dirty(&s) && st.is_dirty());
        assert_eq!(st.value(&s), "B");
        st.edit(&s, "A".into());
        assert!(!st.dirty(&s) && !st.is_dirty());
    }

    #[test]
    fn clear_saved_keeps_newer_input() {
        let mut st = State::default();
        let s = spec();
        st.edit(&s, "B".into());
        st.sent.insert(s.id.clone(), "B".into());
        st.edit(&s, "BC".into());
        st.clear_saved(&Op::Patch("account".into()));
        assert_eq!(
            st.value(&s),
            "BC",
            "newer input after sending is not erased"
        );
        st.sent.insert(s.id.clone(), "BC".into());
        st.clear_saved(&Op::Patch("account".into()));
        assert!(!st.dirty(&s));
    }
}
