//! View and messages of `SessionSettingsController` (S6-09, S6-10): session settings, permissions, tasks, confirmations.

use crate::permissions::{Mode, PermTable};
use crate::tool_types::{RunItem, TaskItem};
use crate::tree_types::{PersonaItem, SessionItem, TreeFrame};

/// Actions whose permissions are configured at session level.
pub const PERM_ACTIONS: [&str; 3] = ["send", "delete", "shell"];

/// Session model: account (`account_id`) with a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub account_id: i64,
    pub label: String,
}

/// Session notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotifMode {
    #[default]
    All,
    Important,
    Off,
}

impl NotifMode {
    pub fn as_str(self) -> &'static str {
        match self {
            NotifMode::All => "all",
            NotifMode::Important => "important",
            NotifMode::Off => "off",
        }
    }

    pub fn parse(s: Option<&str>) -> NotifMode {
        match s {
            Some("important") => NotifMode::Important,
            Some("off") => NotifMode::Off,
            _ => NotifMode::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SsMsg {
    /// A session is open (`None` - closed).
    Open(Option<i64>),
    Snapshot(Vec<SessionItem>),
    Tree(TreeFrame),
    Personas(Vec<PersonaItem>),
    Models(Vec<ModelChoice>),
    Perms(PermTable),
    /// The core rejected the permission write (loosening, `403`): the permission list is re-read.
    PermRejected,
    Tasks {
        session_id: i64,
        items: Vec<TaskItem>,
    },
    Runs {
        task_id: i64,
        items: Vec<RunItem>,
    },
    /// New or changed task from the core reply.
    TaskChanged(TaskItem),
    SetPersona(Option<i64>),
    SetPath(String),
    MoveTo(Option<i64>),
    /// Parent context of a thread: `true` - inherit, `false` - isolated.
    SetCtxInherit(bool),
    SetModel(Option<i64>),
    SetMem(bool),
    SetAuto(bool),
    SetNotif(NotifMode),
    /// Session permission: `None` - "as persona".
    SetPerm {
        action: String,
        mode: Option<Mode>,
    },
    NewThread,
    AskClear,
    AskDelete,
    Export,
    Exported {
        path: String,
        size: i64,
    },
    Confirm,
    Cancel,
    TaskCreate {
        kind: String,
        title: String,
        spec: String,
        cron: Option<String>,
    },
    TaskPause(i64),
    TaskResume(i64),
    TaskRun(i64),
    ShowRuns(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    Clear,
    Delete,
}

/// Confirmation listing what will be lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmView {
    pub kind: ConfirmKind,
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermOption {
    pub mode: Mode,
    pub label: String,
    /// `false` - the value loosens the higher-level rule.
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermField {
    pub action: String,
    pub label: String,
    /// Explicit session value; `None` - "as persona".
    pub selected: Option<Mode>,
    /// The strictest of the three levels.
    pub effective: Mode,
    pub options: Vec<PermOption>,
    /// A higher level forbids: nothing to change.
    pub locked: bool,
    /// Explanation for unavailable values.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub state: String,
    /// "cron · zone" for `sched`, empty for `deleg`.
    pub schedule: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SsView {
    pub session_id: Option<i64>,
    pub name: String,
    pub is_thread: bool,
    pub persona: Option<i64>,
    pub personas: Vec<(i64, String)>,
    pub path: String,
    pub folder_id: Option<i64>,
    pub inherit_ctx: bool,
    pub model: Option<i64>,
    pub models: Vec<ModelChoice>,
    pub mem: bool,
    pub auto: bool,
    pub notif: NotifMode,
    pub perms: Vec<PermField>,
    pub tasks: Vec<TaskRow>,
    /// The task whose latest runs are shown, and the runs themselves.
    pub runs_of: Option<i64>,
    pub runs: Vec<RunItem>,
    pub confirm: Option<ConfirmView>,
    pub notice: Option<String>,
    pub exported: Option<String>,
}
