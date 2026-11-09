//! Sending from the input field: draft → `SendMessage`, a placeholder bubble until the core confirms.

use serde_json::Value;

use crate::chat_attach::preview;
use crate::chat_bubbles::bubble;
use crate::chat_types::{Message, MsgStatus, Role};
use crate::chat_view::{Bubble, Delivery};
use crate::controller::{CoreCommand, Effect};
use crate::controllers::chat::{ChatController, Pending};
use crate::i18n::tr;

impl ChatController {
    pub(crate) fn submit(&mut self, client_id: String) -> Vec<Effect> {
        let Some(session_id) = self.session else {
            return Vec::new();
        };
        let body = self.draft().trim_end().to_string();
        let attachments = self.staged().to_vec();
        if body.trim().is_empty() && attachments.is_empty() {
            return Vec::new();
        }
        self.drafts.remove(&session_id);
        self.staged.remove(&session_id);
        self.pending.push(Pending {
            session_id,
            client_id: client_id.clone(),
            body: body.clone(),
            attachments: attachments
                .iter()
                .map(|a| preview(&a.name, a.size))
                .collect(),
            failed: false,
        });
        self.at_bottom = true;
        self.scroll_seq += 1;
        vec![Effect::Command(CoreCommand::SendMessage {
            session_id,
            client_id,
            body_md: body,
            attachments,
        })]
    }

    pub(crate) fn pending_bubble(&self, p: &Pending) -> Bubble {
        let mut b = bubble(
            &Message {
                id: 0,
                client_id: Some(p.client_id.clone()),
                role: Role::User,
                author: None,
                body: p.body.clone(),
                status: MsgStatus::Done,
                meta: Value::Null,
                ts: 0,
            },
            None,
            self.utc_offset,
        );
        b.id = None;
        b.time = String::new();
        if p.failed {
            b.delivery = Delivery::Failed;
            b.note = Some(tr("chat-send-failed"));
        } else {
            b.delivery = Delivery::Pending;
        }
        b.attachments = p.attachments.clone();
        b
    }
}
