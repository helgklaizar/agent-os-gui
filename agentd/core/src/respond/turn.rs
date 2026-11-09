//! Agent turn: a model reply, then the actions it asks for, and new replies with their results.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Notify;

use super::stream::{Round, one_round};
use super::{Inner, TurnEnd, dispatch, topic};

/// Model rounds per turn: a reply, then tool results and further replies.
const MAX_ROUNDS: usize = 4;
/// Tool calls the core takes from a single reply.
const MAX_CALLS: usize = 4;

/// Turn: a model reply; if it asks for actions, they go through the dispatcher, results return to the model, and so on until
/// a reply without calls.
pub(super) async fn respond(
    inner: &Arc<Inner>,
    session_id: i64,
    user_msg_id: i64,
    stop: Arc<Notify>,
    run_as: super::RunAs,
) -> TurnEnd {
    let mut expired = false;
    let mut upto = user_msg_id;
    let actor = run_as.actor;
    let mut untrusted: Option<String> = run_as.untrusted;
    let mut last_status = "done";
    let mut last_text = String::new();
    'turn: for round in 0..MAX_ROUNDS {
        let Some(Round {
            status,
            calls,
            ctx,
            text,
        }) = one_round(inner, session_id, upto, &stop).await
        else {
            return TurnEnd {
                status: "error",
                expired,
                text: last_text,
            };
        };
        last_status = status;
        last_text = text;
        if status != "done" || calls.is_empty() || round + 1 == MAX_ROUNDS {
            break;
        }
        let mut last_id = None;
        for call in calls.iter().take(MAX_CALLS) {
            let rep = dispatch::dispatch(
                inner,
                &ctx,
                session_id,
                call,
                untrusted.as_deref(),
                &stop,
                actor,
            )
            .await;
            expired |= rep.code == "expired";
            if rep.code == "stopped" {
                last_status = "stopped";
                break 'turn;
            }
            // A tool error (`tool_error`, MCP `isError`) also carries untrusted server text.
            if (rep.ok || rep.code == "tool_error") && rep.untrusted_output.is_some() {
                untrusted = Some(rep.origin.clone());
            }
            let Ok(msg) = inner
                .store
                .create_tool(session_id, rep.body(), rep.meta())
                .await
            else {
                // the session was deleted mid-turn
                return TurnEnd {
                    status: "error",
                    expired,
                    text: last_text,
                };
            };
            last_id = msg["id"].as_i64();
            inner.notifier.publish_event(
                &topic(session_id),
                json!({"kind": "message.created", "message": msg}),
            );
        }
        let Some(id) = last_id else { break };
        upto = id;
    }
    let tp = topic(session_id);
    if last_status != "stopped"
        && inner.notifier.watchers(&tp) == 0
        && inner.tree.bump_unread(session_id).await.is_err()
    {
        tracing::debug!(
            scope = "chat",
            event = "unread_skipped",
            "session vanished before the badge"
        );
    } else if last_status != "stopped" && inner.notifier.watchers(&tp) == 0 {
        super::events::agent_replied(inner, session_id).await;
    }
    TurnEnd {
        status: last_status,
        expired,
        text: last_text,
    }
}
