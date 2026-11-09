//! Core document fields on Settings pages: description and value display; input parsing is in `settings_parse`.
//! One description serves both the view and saving: a field missing here never goes into the patch.

use serde_json::Value;

use crate::i18n::tr;
use crate::settings_model::{CoreModel, id_of, text_of};
pub use crate::settings_parse::{Bad, merge, nest, parse};
pub use crate::settings_specs::{page_specs, provider_specs};

/// Field value type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Text(usize),
    Toggle,
    Choice(Vec<(String, String)>),
    /// Choice by record index (persona, account); an empty value is `null`.
    ChoiceId(Vec<(String, String)>),
    Int(i64, i64),
    /// `HH:MM`; an empty value is `null`.
    Time,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub id: String,
    pub doc: String,
    pub path: String,
    pub label: String,
    pub ty: Ty,
    /// What to show while the document is empty.
    pub default: String,
}

/// Document field identifier.
pub fn doc_field_id(doc: &str, path: &str) -> String {
    format!("d:{doc}:{path}")
}

pub(crate) fn spec(doc: &str, path: &str, label: String, ty: Ty, default: &str) -> Spec {
    Spec {
        id: doc_field_id(doc, path),
        doc: doc.to_string(),
        path: path.to_string(),
        label,
        ty,
        default: default.to_string(),
    }
}

pub(crate) fn opts(items: &[(&str, String)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(v, l)| (v.to_string(), l.clone()))
        .collect()
}

pub(crate) fn none_and(items: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), tr("set-none"))];
    out.extend(items);
    out
}

pub(crate) fn persona_options(m: &CoreModel) -> Vec<(String, String)> {
    none_and(
        m.personas
            .iter()
            .filter_map(|p| Some((id_of(p)?.to_string(), text_of(p, "name").to_string())))
            .collect(),
    )
}

pub(crate) fn ai_options(m: &CoreModel) -> Vec<(String, String)> {
    none_and(
        m.ai_accounts()
            .into_iter()
            .filter_map(|a| Some((id_of(a)?.to_string(), text_of(a, "label").to_string())))
            .collect(),
    )
}

/// Display of a value from the core document.
pub fn show(s: &Spec, m: &CoreModel) -> String {
    match m.at(&s.doc, &s.path) {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => match s.ty {
            Ty::Toggle => "false".to_string(),
            _ => s.default.clone(),
        },
    }
}
