//! Port for ending the user session: implemented by `agentd` (`loginctl terminate-session`).

use crate::PortFuture;

pub trait SessionCloser: Send + Sync + 'static {
    /// Starts ending the session; `false` — the session is unknown, nothing was started.
    fn close(&self) -> PortFuture<'_, bool>;
}
