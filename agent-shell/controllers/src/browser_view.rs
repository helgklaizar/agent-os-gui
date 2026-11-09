//! View and messages of `BrowserController`: no logic, only widget data and profile directories.

use std::path::{Path, PathBuf};

use crate::tool_types::{BrowserCommand, Link, ToolResult};

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserMsg {
    /// A session was opened (`None` means closed).
    Session(Option<i64>),
    /// Enter in the address bar.
    Submit(String),
    Back,
    Forward,
    Reload,
    /// "Home": the start page; the application does not reload.
    Home,
    /// WebKit started showing an address (navigation, link, redirect).
    Loaded {
        session_id: i64,
        url: String,
        title: String,
    },
    /// Quick links from the core with `rev` taken from `ETag`.
    Quick {
        items: Vec<Link>,
        rev: i64,
    },
    /// Session bookmarks from the core.
    Marks {
        session_id: i64,
        items: Vec<Link>,
        rev: i64,
    },
    /// "Add" on the start page.
    AddQuick {
        title: String,
        url: String,
    },
    /// Add the current page to bookmarks.
    AddMark,
    RemoveMark(usize),
    /// `412` on write: the list changed in the core, re-read it.
    Stale {
        session_id: Option<i64>,
    },
    /// An agent command to the browser (`browser.command` frame of topic `session.<id>`).
    Agent {
        session_id: i64,
        command: BrowserCommand,
    },
    /// Result of an agent command reported by the browser view.
    AgentDone {
        session_id: i64,
        result: ToolResult,
    },
}

/// Load an address in the session page; the view executes the request when `seq` is newer than the last executed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    pub session_id: i64,
    pub url: String,
    pub seq: u64,
}

/// Load by agent command in a separate profile (not in the user's page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLoad {
    pub session_id: i64,
    pub request_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserView {
    pub session: Option<i64>,
    /// Start page (no address).
    pub start_page: bool,
    /// Address for the input field (no control characters, up to 2048 characters).
    pub address: String,
    pub title: String,
    pub can_back: bool,
    pub can_forward: bool,
    pub load: Option<LoadRequest>,
    pub agent: Option<AgentLoad>,
    pub quick: Vec<Link>,
    pub bookmarks: Vec<Link>,
    /// The current page is already bookmarked.
    pub bookmarked: bool,
}

/// WebKit profile directory of a session: cookies, data and history are not shared between sessions.
pub fn profile_dir(base: &Path, session_id: i64) -> PathBuf {
    base.join("sessions").join(session_id.to_string())
}

/// A separate profile for agent commands: agent pages cannot see the user's cookies.
pub fn agent_profile_dir(base: &Path, session_id: i64) -> PathBuf {
    base.join("agent").join(session_id.to_string())
}
