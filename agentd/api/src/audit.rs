//! Writing to the action log from handlers: token issue and revocation, permission changes, decisions (A4-05).

use agentd_core::{Actor, NewAction, Principal};
use serde_json::Value;

use crate::state::ApiState;

/// Records an owner action (`actor = user`) on behalf of token `who`. A log failure is visible in the core log, not swallowed:
/// the operation itself has already been performed and cannot be rolled back. Token values and hashes are never passed here.
pub(crate) async fn user_action(
    state: &ApiState,
    who: &Principal,
    action: &str,
    target: String,
    args: Value,
) {
    user_action_with(state, who, action, target, args, None, "ok").await;
}

/// Same, with a decision (`deny` for a rejected attempt) and an outcome.
pub(crate) async fn user_action_with(
    state: &ApiState,
    who: &Principal,
    action: &str,
    target: String,
    args: Value,
    decision: Option<agentd_core::Decision>,
    result: &str,
) {
    let Some(audit) = state.services().and_then(|s| s.audit.clone()) else {
        tracing::error!(
            scope = "audit",
            event = "no_audit",
            action,
            "audit log not connected"
        );
        return;
    };
    let mut args = args;
    if let Value::Object(o) = &mut args {
        o.insert("by_token_id".into(), who.token_id.into());
    }
    let entry = NewAction {
        actor: Actor::User,
        session_id: None,
        persona_id: None,
        action: action.to_string(),
        target: Some(target),
        args: Some(args),
        decision,
        result: Some(result.to_string()),
    };
    if audit.record(entry).await.is_err() {
        tracing::error!(
            scope = "audit",
            event = "record_failed",
            action,
            "audit log write failed"
        );
    }
}
