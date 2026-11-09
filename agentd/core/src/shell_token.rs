//! `kind = agent` token for `shell` commands: issued for one command and guaranteed revocation.

use std::sync::Arc;

use crate::PortFuture;

/// `kind = agent` token for a `shell` command: the value is hidden in `Debug`, the runner gets only a descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentToken {
    pub id: i64,
    value: String,
}

impl AgentToken {
    #[must_use]
    pub fn new(id: i64, value: String) -> AgentToken {
        AgentToken { id, value }
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Debug for AgentToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentToken")
            .field("id", &self.id)
            .field("value", &"<hidden>")
            .finish()
    }
}

/// Issuing an agent token to a `shell` command: the token lives for one command and is revoked after it.
pub trait AgentTokens: Send + Sync + 'static {
    /// `None` — the token could not be issued (the command does not start).
    fn issue<'a>(&'a self, session_id: i64) -> PortFuture<'a, Option<AgentToken>>;
    fn revoke<'a>(&'a self, token_id: i64) -> PortFuture<'a, ()>;
}

/// Revokes the command token even if the executor future is dropped (reply stopped): the revoke goes as a separate task.
pub(crate) struct RevokeGuard {
    pub(crate) tokens: Option<Arc<dyn AgentTokens>>,
    pub(crate) id: Option<i64>,
}

impl RevokeGuard {
    pub(crate) async fn revoke_now(mut self) {
        if let (Some(t), Some(id)) = (self.tokens.take(), self.id.take()) {
            t.revoke(id).await;
        }
    }
}

impl Drop for RevokeGuard {
    fn drop(&mut self) {
        if let (Some(t), Some(id)) = (self.tokens.take(), self.id.take())
            && let Ok(h) = tokio::runtime::Handle::try_current()
        {
            h.spawn(async move { t.revoke(id).await });
        }
    }
}
