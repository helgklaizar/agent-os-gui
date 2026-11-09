//! Settings dialog view: navigation, page rows, banner, confirmation. Widgets build themselves from this view,
//! there is no logic in them; all user text is shown escaped (S7, rule 12).

use crate::settings_types::{Action, Close, Secret};
use crate::types::{SettingsPage, Theme};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavItem {
    pub page: SettingsPage,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavGroup {
    pub title: String,
    pub items: Vec<NavItem>,
}

/// Field view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// Write-only: empty after saving, the value is not read or shown.
    Secret,
    Toggle,
    /// The value is the first element of the pair, the label is the second.
    Choice(Vec<(String, String)>),
    Number,
    TextArea,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRow {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub value: String,
    pub error: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowAction {
    pub action: Action,
    pub label: String,
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub title: String,
    pub subtitle: String,
    pub tags: Vec<String>,
    pub actions: Vec<RowAction>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading(String),
    Note(String),
    Field(FieldRow),
    Item(ItemRow),
    Button(RowAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub kind: BannerKind,
    pub text: String,
}

/// Confirmation of a dangerous action listing what will be lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    pub open: bool,
    pub page: SettingsPage,
    pub title: String,
    pub nav: Vec<NavGroup>,
    pub rows: Vec<Row>,
    pub banner: Option<Banner>,
    pub confirm: Option<Confirm>,
    pub dirty: bool,
    pub saving: bool,
    pub loading: bool,
    /// The "Save" and "Close" buttons are always visible: the dialog never hides them in any state.
    pub save_visible: bool,
    pub close_visible: bool,
}

/// Message to the controller.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsMsg {
    /// Bus event: `OpenSettings(page)`.
    Bus(crate::bus::BusEvent),
    Open(SettingsPage),
    Close(Close),
    Select(SettingsPage),
    Edit {
        id: String,
        value: String,
    },
    EditSecret {
        id: String,
        value: Secret,
    },
    Save,
    Act(Action),
    Confirm(bool),
    Reply(crate::settings_types::Reply),
    /// `settings` frame from the core: another client changed the page.
    Changed {
        page: String,
        rev: i64,
    },
    /// `bar.quota` snapshot (the same as the bar's).
    Quotas(Vec<crate::bar_types::QuotaItem>),
    Sys(crate::bar_types::SysFrame),
    /// Theme the shell applies right now.
    ThemeNow(Theme),
    /// "System" and "Configuration" rows for About.
    Env {
        system: String,
        config: String,
    },
}
