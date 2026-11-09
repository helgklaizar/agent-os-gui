//! Who requested a session reply and how the turn ended: needed by task and trigger runs.

use crate::Actor;

/// Who requested the reply and whom to tell how it ended (a task run is `Actor::Cron`).
pub(super) struct Origin {
    pub run_as: RunAs,
    pub done: Option<tokio::sync::oneshot::Sender<TurnEnd>>,
}

/// On whose behalf the agent turn runs and whether its context is untrusted (a trigger run: the incoming
/// message text is external, turn actions require cards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAs {
    pub actor: Actor,
    /// Untrusted text source from the very start of the turn (`origin` of the permission card).
    pub untrusted: Option<String>,
}

impl RunAs {
    /// Normal turn from a user message.
    #[must_use]
    pub fn agent() -> RunAs {
        RunAs {
            actor: Actor::Agent,
            untrusted: None,
        }
    }

    /// Scheduled task run.
    #[must_use]
    pub fn cron() -> RunAs {
        RunAs {
            actor: Actor::Cron,
            untrusted: None,
        }
    }

    /// Trigger run: the context is untrusted.
    #[must_use]
    pub fn trigger(origin: &str) -> RunAs {
        RunAs {
            actor: Actor::Cron,
            untrusted: Some(origin.to_string()),
        }
    }
}

/// Limit on the reply text a turn gives to a task run (`task_runs.output`), in bytes.
const OUTPUT_MAX: usize = 4000;

/// Appends `add` to `buf` without exceeding `OUTPUT_MAX` or splitting a character.
pub(super) fn push_capped(buf: &mut String, add: &str) {
    let room = OUTPUT_MAX.saturating_sub(buf.len());
    let mut end = add.len().min(room);
    while !add.is_char_boundary(end) {
        end -= 1;
    }
    buf.push_str(&add[..end]);
}

/// Agent turn outcome for a task run: reply status, the "permission request expired" flag and the start of the last reply text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnd {
    pub status: &'static str,
    pub expired: bool,
    /// Start of the last model reply text (up to `OUTPUT_MAX` bytes); empty if there is no reply.
    pub text: String,
}
