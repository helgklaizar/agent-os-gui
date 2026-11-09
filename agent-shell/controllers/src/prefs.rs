//! `ui.json` format (no I/O: the file is read and written by `ui`).

use serde::{Deserialize, Serialize};

use crate::types::{Manual, Theme};

/// Current `ui.json` format version.
pub const UI_JSON_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColumnPref {
    #[serde(with = "manual_serde")]
    pub manual: Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColumnsPrefs {
    pub notifications: ColumnPref,
    pub sessions: ColumnPref,
    pub tools: ColumnPref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPrefs {
    pub w: u32,
    pub h: u32,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self { w: 1280, h: 800 }
    }
}

/// The whole `ui.json` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiFile {
    pub version: u32,
    pub theme: Theme,
    pub columns: ColumnsPrefs,
    pub window: WindowPrefs,
}

impl Default for UiFile {
    fn default() -> Self {
        Self {
            version: UI_JSON_VERSION,
            theme: Theme::default(),
            columns: ColumnsPrefs::default(),
            window: WindowPrefs::default(),
        }
    }
}

impl UiFile {
    /// Parse the file. Broken JSON or a foreign version yields defaults and a reason for the log:
    /// startup does not fail because of it.
    pub fn parse_or_default(text: &str) -> (Self, Option<String>) {
        match Self::parse(text) {
            Ok(file) => (file, None),
            Err(reason) => (Self::default(), Some(reason)),
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("ui.json: parse error: {e}"))?;
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(v) if v == u64::from(UI_JSON_VERSION) => {}
            Some(v) => return Err(format!("ui.json: unknown version {v}")),
            None => return Err("ui.json: no version field".to_string()),
        }
        serde_json::from_value(value).map_err(|e| format!("ui.json: parse error: {e}"))
    }

    pub fn to_json(&self) -> String {
        // serialising simple structs without non-string keys does not fail
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Merge in a partial update.
    pub fn apply(&mut self, p: &UiPrefs) {
        if let Some(theme) = p.theme {
            self.theme = theme;
        }
        if let Some(columns) = p.columns {
            self.columns = columns;
        }
        if let Some(window) = p.window {
            self.window = window;
        }
    }
}

/// Partial `ui.json` update: `Some` writes, `None` leaves untouched.
/// Needed so that `ThemeController` and `ShellController` do not overwrite each other's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiPrefs {
    pub theme: Option<Theme>,
    pub columns: Option<ColumnsPrefs>,
    pub window: Option<WindowPrefs>,
}

/// `null` <-> `Manual::Auto`, `"collapsed"`, `"expanded"`.
mod manual_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::types::Manual;

    pub fn serialize<S: Serializer>(m: &Manual, s: S) -> Result<S::Ok, S::Error> {
        match m {
            Manual::Auto => None::<&str>.serialize(s),
            Manual::Collapsed => Some("collapsed").serialize(s),
            Manual::Expanded => Some("expanded").serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Manual, D::Error> {
        match Option::<String>::deserialize(d)?.as_deref() {
            None => Ok(Manual::Auto),
            Some("collapsed") => Ok(Manual::Collapsed),
            Some("expanded") => Ok(Manual::Expanded),
            Some(other) => Err(serde::de::Error::custom(format!(
                "manual: unknown value {other:?}"
            ))),
        }
    }
}
