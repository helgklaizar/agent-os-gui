//! A `kind = agent` token for the `shell` command (A7-02): issued for one command, the value goes to the runner as a descriptor,
//! revoked after the command (WS of this token are closed with `4401`). The issue is recorded in the action journal without the value.

use std::sync::Arc;

use agentd_api::Hub;
use agentd_auth::TokenStore;
use agentd_core::{Actor, AgentToken, AgentTokens, Audit, Decision, NewAction, PortFuture};

pub struct ShellTokens {
    pub store: Arc<TokenStore>,
    pub hub: Hub,
    pub audit: Audit,
}

impl AgentTokens for ShellTokens {
    fn issue<'a>(&'a self, session_id: i64) -> PortFuture<'a, Option<AgentToken>> {
        Box::pin(async move {
            let issued = self.store.issue_agent(session_id).await.ok()?;
            let entry = NewAction {
                actor: Actor::User,
                session_id: Some(session_id),
                persona_id: None,
                action: "token_issue".to_string(),
                target: Some(format!("agent:{session_id}")),
                args: None,
                decision: Some(Decision::Allow),
                result: Some("ok".to_string()),
            };
            if self.audit.record(entry).await.is_err() {
                tracing::error!(
                    scope = "audit",
                    event = "record_failed",
                    "token issue not recorded in the journal"
                );
            }
            Some(AgentToken::new(issued.id, issued.value.expose().clone()))
        })
    }

    fn revoke<'a>(&'a self, token_id: i64) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if self.store.revoke(token_id).await.is_err() {
                tracing::error!(
                    scope = "auth",
                    event = "shell_token_revoke_failed",
                    "command token not revoked"
                );
            }
            self.hub.revoke_token(token_id);
        })
    }
}

/// `shell` executor with an agent token per command.
pub fn executor(
    runner: Arc<crate::runner_client::RunnerClient>,
    tree: Arc<dyn agentd_core::TreeStore>,
    (store, hub, audit): (&Arc<TokenStore>, Hub, Audit),
) -> Arc<agentd_core::ShellExecutor> {
    let notifier = Arc::new(hub.clone());
    let tokens = Arc::new(ShellTokens {
        store: store.clone(),
        hub,
        audit,
    });
    Arc::new(agentd_core::ShellExecutor::new(runner, tree, notifier).with_tokens(tokens))
}
