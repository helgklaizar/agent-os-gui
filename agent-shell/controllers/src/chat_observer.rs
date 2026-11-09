//! Observer confirmation cards: red frame, finding, verdict, confirmation of a dangerous action.

use crate::chat_cards::{button_label, state_label};
use crate::chat_types::PermRequest;
use crate::chat_view::{
    ButtonKind, CardButton, CardState, ChatItem, ObserverCard, ObserverKind, Verdict,
};
use crate::controllers::chat::ChatController;
use crate::i18n::tr;
use crate::secrets::mask_secrets;

fn kind_of(kind: Option<&str>) -> (ObserverKind, String) {
    match kind {
        Some("injection") => (ObserverKind::Injection, tr("chat-observer-injection")),
        Some("phishing") => (ObserverKind::Phishing, tr("chat-observer-phishing")),
        Some("dangerous_cmd") => (
            ObserverKind::DangerousCmd,
            tr("chat-observer-dangerous-cmd"),
        ),
        Some("module") => (ObserverKind::Module, tr("chat-observer-module")),
        Some("permission_loosen") => (ObserverKind::PermissionLoosen, tr("chat-observer-loosen")),
        _ => (ObserverKind::Other, tr("chat-observer-other")),
    }
}

/// An unknown verdict is read as `blocked`: no buttons, the action is closed (fail-closed).
fn verdict_of(verdict: Option<&str>) -> (Verdict, String) {
    match verdict {
        Some("asked") => (Verdict::Asked, tr("chat-verdict-asked")),
        Some("allowed") => (Verdict::Allowed, tr("chat-verdict-allowed")),
        _ => (Verdict::Blocked, tr("chat-verdict-blocked")),
    }
}

impl ChatController {
    pub(crate) fn card_item(&self, req: &PermRequest) -> ChatItem {
        if req.observer {
            self.observer_item(req)
        } else {
            self.ask_item(req)
        }
    }

    fn observer_item(&self, req: &PermRequest) -> ChatItem {
        let (kind, kind_label) = kind_of(req.kind.as_deref());
        let (verdict, verdict_label) = verdict_of(req.verdict.as_deref());
        let state = self.card_state(req);
        let awaiting = verdict == Verdict::Asked && state == CardState::Pending;
        ChatItem::Observer(ObserverCard {
            request_id: req.id,
            kind,
            kind_label,
            target: mask_secrets(&req.target),
            detail: req.detail.as_deref().map(mask_secrets),
            verdict,
            verdict_label,
            red_border: true,
            awaiting,
            state,
            state_label: state_label(state),
            buttons: if awaiting {
                [ButtonKind::Confirm, ButtonKind::Reject]
                    .map(|kind| CardButton {
                        kind,
                        label: button_label(kind),
                    })
                    .to_vec()
            } else {
                Vec::new()
            },
        })
    }
}
