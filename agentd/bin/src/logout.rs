//! Ending the user's session: `loginctl terminate-session $XDG_SESSION_ID` with an empty environment. With no session
//! identifier the command is not run (ending all of the user's sessions blindly is not allowed).

use agentd_core::{PortFuture, SessionCloser};
use tokio::process::Command;

pub(crate) struct Loginctl;

/// A logind session identifier: letters and digits only.
fn session_id() -> Option<String> {
    std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric()))
}

impl SessionCloser for Loginctl {
    fn close(&self) -> PortFuture<'_, bool> {
        Box::pin(async move {
            let Some(id) = session_id() else { return false };
            let program = if agentd_core::manual_clock(0).is_some() {
                std::env::var("AGENTD_TUNNEL_SSH").unwrap_or_else(|_| "loginctl".into())
            } else {
                "loginctl".to_string()
            };
            let mut c = Command::new(program);
            c.args(["terminate-session", &id])
                .env_clear()
                .env("PATH", "/usr/local/bin:/usr/bin:/bin")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if agentd_core::manual_clock(0).is_some()
                && let Some(p) = std::env::var_os("TUNNEL_FAKE_LOG")
            {
                c.env("TUNNEL_FAKE_LOG", p);
            }
            c.spawn().is_ok()
        })
    }
}
