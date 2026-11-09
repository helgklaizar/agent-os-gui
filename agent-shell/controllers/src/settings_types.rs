//! Settings dialog types (S7): pages and groups, core commands, replies, row actions.
//!
//! Field ids are strings: `d:<core page>:<path>` (document field), `p:<id|new>:<field>` (persona),
//! `s:<ref>` (secret, write-only), `t:new:<field>` (tunnel), `a:new:<field>` (account), `m:url`, `f:text`, `theme`.

use serde_json::Value;

use crate::permissions::Mode;
use crate::types::SettingsPage;

/// Left navigation group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    General,
    Ai,
    System,
}

/// Pages by group in display order.
pub const NAV: [(Group, &[SettingsPage]); 3] = [
    (
        Group::General,
        &[
            SettingsPage::Account,
            SettingsPage::Personas,
            SettingsPage::Connections,
            SettingsPage::Indicators,
        ],
    ),
    (
        Group::Ai,
        &[
            SettingsPage::Models,
            SettingsPage::Modules,
            SettingsPage::Permissions,
        ],
    ),
    (
        Group::System,
        &[
            SettingsPage::Security,
            SettingsPage::Schedule,
            SettingsPage::Tunnels,
            SettingsPage::Files,
            SettingsPage::About,
        ],
    ),
];

/// Settings root: the first page.
pub fn resolve(page: SettingsPage) -> SettingsPage {
    match page {
        SettingsPage::Root => SettingsPage::Account,
        other => other,
    }
}

/// Core pages the dialog needs (`GET /settings/{page}`).
pub const DOCS: [&str; 11] = [
    "account",
    "about",
    "personas",
    "models",
    "connections",
    "permissions",
    "security",
    "schedule",
    "memory",
    "tunnels",
    "indicators",
];

/// Secret: the value goes only into the request; it is absent from `Debug` (logs and panic messages do not show the input).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// Value to send to the core; not read anywhere else.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

/// Command to the core. Executed by the layer above (`ui` + `core-client`); the reply arrives as [`Reply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Read all pages and lists (dialog open).
    FetchAll,
    /// `GET /v1/settings/{doc}`.
    FetchPage(String),
    /// `PATCH /v1/settings/{doc}` with `If-Match: <rev>`.
    PatchPage {
        doc: String,
        rev: i64,
        patch: Value,
    },
    /// `PUT /v1/secrets/{ref}`.
    PutSecret {
        reference: String,
        value: Secret,
    },
    CreatePersona(Value),
    PatchPersona {
        id: i64,
        body: Value,
    },
    DeletePersona(i64),
    CreateAccount(Value),
    DeleteAccount(i64),
    ConnectAccount(i64),
    InstallModule(Value),
    PatchModule {
        id: String,
        body: Value,
    },
    DeleteModule(String),
    CreateTunnel(Value),
    PatchTunnel {
        id: i64,
        body: Value,
    },
    DeleteTunnel(i64),
    PutPermission {
        scope: String,
        scope_id: i64,
        action: String,
        mode: Mode,
    },
    FetchFile(String),
    PutFile {
        path: String,
        rev: String,
        text: String,
    },
}

/// What exactly failed (to show the error on the right page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Patch(String),
    Secret(String),
    Persona,
    Account,
    Module,
    Tunnel,
    Permission,
    File,
}

/// Core error without texts: code, fields (paths) and flags for display.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveError {
    pub status: u16,
    pub code: String,
    /// Field paths from `details.fields` (`422`).
    pub fields: Vec<String>,
}

impl SaveError {
    pub fn stale(&self) -> bool {
        self.status == 412
    }
}

/// Editor file list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub size: i64,
    pub rev: String,
}

/// Core reply to a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// Page was read or saved.
    Page {
        doc: String,
        value: Value,
        rev: i64,
        saved: bool,
    },
    Personas(Vec<Value>),
    Accounts(Vec<Value>),
    Modules(Vec<Value>),
    Tunnels(Vec<Value>),
    Files(Vec<FileEntry>),
    Secrets(Vec<String>),
    Perms(Value),
    File {
        path: String,
        text: String,
        rev: String,
    },
    FileSaved {
        path: String,
        rev: String,
    },
    /// Operation done, the lists must be re-read.
    Done(Op),
    /// Module installation awaits a decision (observer check).
    ModulePending,
    Failed {
        op: Op,
        error: SaveError,
    },
}

/// How the dialog was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    Esc,
    Button,
    Backdrop,
}

/// Action on a row or a page button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    PersonaNew,
    /// Show the new persona form.
    PersonaFormNew,
    PersonaSelect(i64),
    PersonaDelete(i64),
    AccountAdd,
    AccountConnect(i64),
    AccountDelete(i64),
    McpAdd,
    McpToggle(usize),
    McpRemove(usize),
    ModuleInstall(String),
    ModuleInstallUrl,
    ModuleToggle(String),
    ModuleRemove(String),
    TunnelAdd,
    TunnelToggle(i64),
    TunnelDelete(i64),
    IndicatorToggle(String),
    IndicatorUp(String),
    IndicatorDown(String),
    FileOpen(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_hides_value() {
        let s = Secret::new("sk-secret-123");
        let shown = format!(
            "{s:?} {:?}",
            Cmd::PutSecret {
                reference: "account.3".into(),
                value: s.clone(),
            }
        );
        assert!(!shown.contains("123"), "{shown}");
        assert_eq!(s.expose(), "sk-secret-123");
    }

    #[test]
    fn root_resolves_to_first_page() {
        assert_eq!(resolve(SettingsPage::Root), SettingsPage::Account);
        assert_eq!(resolve(SettingsPage::Models), SettingsPage::Models);
    }

    #[test]
    fn nav_lists_each_page_once_without_root() {
        let all: Vec<_> = NAV.iter().flat_map(|(_, p)| p.iter().copied()).collect();
        assert_eq!(all.len(), 12);
        assert!(!all.contains(&SettingsPage::Root));
        for p in &all {
            assert_eq!(all.iter().filter(|x| *x == p).count(), 1);
        }
    }
}
