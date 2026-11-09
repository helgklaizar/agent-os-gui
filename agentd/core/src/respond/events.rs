//! Feed event sources during a reply: an agent reply in an inactive session (`ai.agent`) and a provider failure.

use crate::{EventLevel, NewEvent, ProviderKind};

use super::Inner;

/// Failure subtype: cloud providers are `claude`, local and test ones are `local`.
fn provider_sub(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic | ProviderKind::OpenAiCompat => "claude",
        ProviderKind::Local | ProviderKind::Scripted => "local",
    }
}

pub(super) async fn agent_replied(inner: &Inner, session_id: i64) {
    if let Some(ev) = &inner.events {
        let e = NewEvent::new(
            "ai",
            "agent",
            EventLevel::Info,
            "The agent replied in a session while it was closed",
        )
        .session(Some(session_id));
        ev.emit(e).await;
    }
}

/// The text holds only the provider error code (a constant), with no provider response details.
pub(super) async fn provider_failed(
    inner: &Inner,
    session_id: i64,
    kind: ProviderKind,
    code: &str,
) {
    if let Some(ev) = &inner.events {
        let e = NewEvent::new(
            "ai",
            provider_sub(kind),
            EventLevel::Error,
            format!("Model error: {code}"),
        )
        .session(Some(session_id));
        ev.emit(e).await;
    }
}
