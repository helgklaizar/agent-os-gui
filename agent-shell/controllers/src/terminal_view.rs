//! View and messages of `TerminalController` (S6-07, S6-08): the user's terminal (VTE) per session and the agent output panel.
//! Also pure functions: working directory, user terminal environment, shell.

use std::path::{Path, PathBuf};

use crate::tool_types::TermFrame;
use crate::tree_types::{SessionItem, TreeFrame};

#[derive(Debug, Clone, PartialEq)]
pub enum TermMsg {
    /// A session is open (`None` - closed).
    Session(Option<i64>),
    /// All sessions (`sessions` snapshot): the session path sets the terminal directory.
    Snapshot(Vec<SessionItem>),
    Tree(TreeFrame),
    /// The view started a shell in this session's VTE.
    Spawned(i64),
    /// The shell in this session's VTE exited.
    Exited(i64),
    /// Agent terminal frame from the core (`terminal.command`, `terminal.output`, `terminal.status`).
    Agent {
        session_id: i64,
        frame: TermFrame,
    },
}

/// A session's user terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTermView {
    pub session_id: i64,
    /// Directory the shell runs or will run in.
    pub cwd: PathBuf,
    pub running: bool,
    /// The view must start the shell (not started yet or exited).
    pub needs_spawn: bool,
}

/// Agent command in the "Agent output" panel (read-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRun {
    pub request_id: String,
    pub command: String,
    pub cwd: String,
    /// `queued`, `started`, `exited`, `killed`, `denied`, `timeout`.
    pub status: String,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub output: String,
    /// Accumulated output hit 256 KiB, the rest is not shown.
    pub truncated: bool,
    /// Next expected output `seq`: a redelivered chunk is ignored.
    pub next_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalView {
    pub session: Option<i64>,
    pub user: Option<UserTermView>,
    /// Sessions with a live shell: the view does not destroy their VTE on switching.
    pub live: Vec<i64>,
    /// Agent commands of the active session, oldest first.
    pub agent: Vec<AgentRun>,
}

/// Accumulated output of one agent command is at most this (api.md section 4.4).
pub const AGENT_OUTPUT_LIMIT: usize = 256 * 1024;
/// How many recent agent commands are remembered per session.
pub const AGENT_RUNS_KEPT: usize = 20;

/// Terminal working directory: the session path, otherwise the home directory. A relative path or a path with `..` is rejected.
pub fn term_cwd(session_path: Option<&str>, home: &Path) -> PathBuf {
    let usable = session_path.map(str::trim).map(Path::new).filter(|p| {
        p.is_absolute() && !p.components().any(|c| c == std::path::Component::ParentDir)
    });
    usable.map_or_else(|| home.to_path_buf(), Path::to_path_buf)
}

/// Names the user terminal inherits from the GUI. Everything else (tokens, `AGENTD_*`, keys, agent sockets,
/// `LD_PRELOAD`, bus and display addresses) is not inherited: the user terminal gets no core privileges (D29).
const INHERITED: [&str; 8] = [
    "HOME", "USER", "LOGNAME", "SHELL", "PATH", "LANG", "LANGUAGE", "TZ",
];

const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// Prompt with the path (`\w`); the shell may replace it with its own settings file.
const PROMPT: &str = "\\[\\e[32m\\]\\w\\[\\e[0m\\] \\$ ";

/// User terminal environment from the GUI environment: only the safe set, no tokens or core descriptors.
pub fn user_terminal_env(parent: &[(String, String)], cwd: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = parent
        .iter()
        .filter(|(k, v)| {
            (INHERITED.contains(&k.as_str()) || k.starts_with("LC_")) && !v.contains('\0')
        })
        .cloned()
        .collect();
    if !out.iter().any(|(k, _)| k == "PATH") {
        out.push(("PATH".into(), DEFAULT_PATH.into()));
    }
    out.push(("TERM".into(), "xterm-256color".into()));
    out.push(("COLORTERM".into(), "truecolor".into()));
    out.push(("PWD".into(), cwd.to_string_lossy().into_owned()));
    out.push(("PS1".into(), PROMPT.into()));
    out
}

/// User shell: `$SHELL` if it is an absolute path without spaces, otherwise `/bin/sh`.
pub fn user_shell(shell_env: Option<&str>) -> String {
    shell_env
        .filter(|s| s.starts_with('/') && !s.contains(char::is_whitespace))
        .unwrap_or("/bin/sh")
        .to_string()
}
