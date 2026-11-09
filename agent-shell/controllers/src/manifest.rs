//! Module manifest (ADR-0004): parsing into types and validation before rendering.
//!
//! Validation rules live in one place: [`crate::module_manifest::validate`] (a copy of the core rules, schema
//! `fake-agentd/schemas/module-manifest.json`); this file only parses and produces a readable user-facing error.

use serde_json::{Map, Value};

use crate::i18n::{tr, tr_args};
use crate::module_manifest::validate;

/// Module placement in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Icon tab in the tools column; the content is a WebView.
    Panel,
    /// Indicator in the bar after the standard ones.
    Widget,
    /// Button in the sessions column.
    Button,
}

/// Validated manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    pub placement: Placement,
    pub permissions: Vec<String>,
    pub network: Vec<String>,
    pub fs: Vec<String>,
    pub questions: Vec<Value>,
}

/// Rejection reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    NotJson,
    NotObject,
    TooLarge,
    /// Fields that failed validation (`id`, `permissions[0]`, name of an unknown field…), in validation order.
    Invalid(Vec<String>),
}

/// Maximum manifest text size.
pub const MAX_BYTES: usize = 64 * 1024;

impl ManifestError {
    /// User-facing text: strings from resources; from foreign text only schema field names get into it.
    pub fn message(&self) -> String {
        match self {
            ManifestError::NotJson => tr("module-manifest-not-json"),
            ManifestError::NotObject => tr("module-manifest-not-object"),
            ManifestError::TooLarge => tr("module-manifest-too-large"),
            ManifestError::Invalid(fields) => {
                tr_args("module-manifest-invalid", &[("fields", &fields.join(", "))])
            }
        }
    }
}

/// Parse and validate a manifest from text.
pub fn parse_manifest(json: &str) -> Result<Manifest, ManifestError> {
    if json.len() > MAX_BYTES {
        return Err(ManifestError::TooLarge);
    }
    let value: Value = serde_json::from_str(json).map_err(|_| ManifestError::NotJson)?;
    manifest_from_value(&value)
}

fn strings(o: &Map<String, Value>, key: &str) -> Vec<String> {
    o.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn text(o: &Map<String, Value>, key: &str) -> String {
    o.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Parse and validate a manifest from a JSON value. The rules live only in [`validate`].
pub fn manifest_from_value(value: &Value) -> Result<Manifest, ManifestError> {
    let Some(o) = value.as_object() else {
        return Err(ManifestError::NotObject);
    };
    let bad = validate(value);
    if !bad.is_empty() {
        return Err(ManifestError::Invalid(bad));
    }
    let placement = match o.get("placement").and_then(Value::as_str) {
        Some("panel") => Placement::Panel,
        Some("widget") => Placement::Widget,
        Some("button") => Placement::Button,
        // validate lets only three placements through; unreachable, but a rejection beats a panic
        _ => return Err(ManifestError::Invalid(vec!["placement".to_string()])),
    };
    Ok(Manifest {
        id: text(o, "id"),
        name: text(o, "name"),
        version: text(o, "version"),
        entry: text(o, "entry"),
        placement,
        permissions: strings(o, "permissions"),
        network: strings(o, "network"),
        fs: strings(o, "fs"),
        questions: o
            .get("questions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    })
}
