//! Reply stream: chunks by `offset`, completion, stop (docs/reference/api.md, section 4.4).

use crate::chat_types::MsgStatus;
use crate::controller::{CoreCommand, Effect};
use crate::controllers::chat::ChatController;

impl ChatController {
    /// Whether a reply is in progress: there is a message with status `streaming`.
    pub(crate) fn is_streaming(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.status == MsgStatus::Streaming)
    }

    /// Requests a snapshot once; until it arrives, further mismatches are silent.
    fn resync_once(&mut self) -> Vec<Effect> {
        let Some(session_id) = self.session.filter(|_| !self.resync) else {
            return Vec::new();
        };
        self.resync = true;
        vec![Effect::Command(CoreCommand::ResyncSession { session_id })]
    }

    /// A delta is applied only if `offset` equals the byte count of the accumulated text.
    pub(crate) fn on_delta(&mut self, message_id: i64, offset: usize, text: &str) -> Vec<Effect> {
        if self.resync {
            return Vec::new();
        }
        let Some(m) = self.messages.iter_mut().find(|m| m.id == message_id) else {
            return self.resync_once();
        };
        if m.body.len() != offset {
            return self.resync_once();
        }
        m.body.push_str(text);
        self.follow();
        Vec::new()
    }

    pub(crate) fn on_done(
        &mut self,
        message_id: i64,
        status: MsgStatus,
        final_len: Option<usize>,
    ) -> Vec<Effect> {
        let Some(m) = self.messages.iter_mut().find(|m| m.id == message_id) else {
            return self.resync_once();
        };
        m.status = status;
        let lost = final_len.is_some_and(|n| n != m.body.len());
        self.follow();
        if lost { self.resync_once() } else { Vec::new() }
    }

    pub(crate) fn stop(&mut self) -> Vec<Effect> {
        match self.session {
            Some(session_id) if self.is_streaming() => {
                vec![Effect::Command(CoreCommand::StopStream { session_id })]
            }
            _ => Vec::new(),
        }
    }
}
