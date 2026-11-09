//! Cards for `ask` permission requests: state, a single answer, "Always" with a level choice.

use crate::chat_types::{Message, PermRequest, ReqStatus, Role};
use crate::chat_view::{
    AlwaysChooser, AskCard, ButtonKind, CardButton, CardState, ChatItem, LevelOption,
};
use crate::controller::{CoreCommand, Effect};
use crate::controllers::chat::ChatController;
use crate::i18n::{tr, tr_args};
use crate::permissions::{Mode, PermDecision, RuleScope, RuleSpec, persona_rule_options};
use crate::secrets::mask_secrets;

/// Request number if the message is a permission card (`meta.kind = "permission_request"`).
pub fn card_request_id(m: &Message) -> Option<i64> {
    if m.role != Role::System || m.meta.get("kind")?.as_str()? != "permission_request" {
        return None;
    }
    m.meta.get("request_id")?.as_i64()
}

fn action_label(action: &str) -> String {
    match action {
        "read" => tr("chat-action-read"),
        "send" => tr("chat-action-send"),
        "delete" => tr("chat-action-delete"),
        "pay" => tr("chat-action-pay"),
        "shell" => tr("chat-action-shell"),
        "browse" => tr("chat-action-browse"),
        "memory_write" => tr("chat-action-memory-write"),
        "config_write" => tr("chat-action-config-write"),
        other => mask_secrets(other),
    }
}

pub(crate) fn button_label(kind: ButtonKind) -> String {
    match kind {
        ButtonKind::Allow => tr("chat-btn-allow"),
        ButtonKind::Deny => tr("chat-btn-deny"),
        ButtonKind::Always => tr("chat-btn-always"),
        ButtonKind::Confirm => tr("chat-btn-confirm"),
        ButtonKind::Reject => tr("chat-btn-reject"),
    }
}

fn button(kind: ButtonKind) -> CardButton {
    CardButton {
        kind,
        label: button_label(kind),
    }
}

pub(crate) fn state_label(state: CardState) -> Option<String> {
    match state {
        CardState::Pending => None,
        CardState::Allowed => Some(tr("chat-card-allowed")),
        CardState::Denied => Some(tr("chat-card-denied")),
        CardState::Always => Some(tr("chat-card-always")),
        CardState::Expired => Some(tr("chat-card-expired")),
    }
}

impl ChatController {
    /// A request from a card message; a resolved request does not return to `pending`.
    pub(crate) fn register_request(&mut self, req: PermRequest) {
        match self.requests.get(&req.id) {
            Some(old) if old.status != ReqStatus::Pending && req.status == ReqStatus::Pending => {}
            _ => {
                self.requests.insert(req.id, req);
            }
        }
    }

    pub(crate) fn register_card(&mut self, m: &Message) {
        if card_request_id(m).is_some()
            && let Some(req) = m.meta.get("request").and_then(PermRequest::from_value)
        {
            self.register_request(req);
        }
    }

    pub(crate) fn resolve_request(&mut self, id: i64, status: ReqStatus) {
        if let Some(r) = self.requests.get_mut(&id) {
            r.status = status;
        }
        if status != ReqStatus::Pending && self.always_open == Some(id) {
            self.always_open = None;
        }
    }

    pub(crate) fn card_state(&self, req: &PermRequest) -> CardState {
        let local = self.local.get(&req.id);
        match (req.status, local) {
            (ReqStatus::Expired, _) => CardState::Expired,
            (ReqStatus::Allowed, Some(PermDecision::Always)) => CardState::Always,
            (ReqStatus::Allowed, _) => CardState::Allowed,
            (ReqStatus::Denied, _) => CardState::Denied,
            (ReqStatus::Pending, Some(PermDecision::Allow)) => CardState::Allowed,
            (ReqStatus::Pending, Some(PermDecision::Deny)) => CardState::Denied,
            (ReqStatus::Pending, Some(PermDecision::Always)) => CardState::Always,
            (ReqStatus::Pending, None) => CardState::Pending,
        }
    }

    fn always_levels(&self, req: &PermRequest) -> Vec<LevelOption> {
        let pick = |modes: &[Mode]| {
            if modes.contains(&Mode::Allow) {
                Mode::Allow
            } else {
                modes[0]
            }
        };
        let mut levels = Vec::new();
        if let Some(p) = self.persona() {
            let modes = persona_rule_options(self.perms.global(&req.action));
            levels.push(LevelOption {
                scope: RuleScope::Persona,
                scope_id: p.id,
                label: tr_args("chat-always-persona", &[("persona", &p.name)]),
                default_mode: pick(&modes),
                modes,
            });
        }
        let modes = vec![Mode::Allow, Mode::Ask, Mode::Deny];
        levels.push(LevelOption {
            scope: RuleScope::Global,
            scope_id: 0,
            label: tr("chat-always-global"),
            default_mode: Mode::Allow,
            modes,
        });
        levels
    }

    pub(crate) fn ask_item(&self, req: &PermRequest) -> ChatItem {
        let state = self.card_state(req);
        let pending = state == CardState::Pending;
        let chooser = (pending && self.always_open == Some(req.id)).then(|| AlwaysChooser {
            levels: self.always_levels(req),
        });
        ChatItem::Ask(AskCard {
            request_id: req.id,
            title: tr_args("chat-ask-title", &[("action", &action_label(&req.action))]),
            target: mask_secrets(&req.target),
            detail: req.detail.as_deref().map(mask_secrets),
            state,
            state_label: state_label(state),
            buttons: if pending {
                [ButtonKind::Allow, ButtonKind::Deny, ButtonKind::Always]
                    .map(button)
                    .to_vec()
            } else {
                Vec::new()
            },
            always: chooser,
        })
    }

    /// A request that can still be answered: waiting and not resolved locally.
    fn open_request(&self, id: i64) -> Option<&PermRequest> {
        self.requests
            .get(&id)
            .filter(|r| r.status == ReqStatus::Pending && !self.local.contains_key(&id))
    }

    fn send_decision(
        &mut self,
        id: i64,
        decision: PermDecision,
        rule: Option<RuleSpec>,
    ) -> Vec<Effect> {
        self.local.insert(id, decision);
        self.always_open = None;
        vec![Effect::Command(CoreCommand::DecidePermission {
            request_id: id,
            decision,
            rule,
        })]
    }

    pub(crate) fn press(&mut self, id: i64, kind: ButtonKind) -> Vec<Effect> {
        let Some(req) = self.open_request(id) else {
            return Vec::new();
        };
        let observer = req.observer;
        match kind {
            ButtonKind::Allow | ButtonKind::Confirm => {
                self.send_decision(id, PermDecision::Allow, None)
            }
            ButtonKind::Deny | ButtonKind::Reject => {
                self.send_decision(id, PermDecision::Deny, None)
            }
            ButtonKind::Always => {
                if !observer {
                    self.always_open = Some(id);
                }
                Vec::new()
            }
        }
    }

    pub(crate) fn always_pick(&mut self, id: i64, scope: RuleScope, mode: Mode) -> Vec<Effect> {
        let Some(req) = self
            .open_request(id)
            .filter(|_| self.always_open == Some(id))
        else {
            return Vec::new();
        };
        let level = self
            .always_levels(req)
            .into_iter()
            .find(|l| l.scope == scope);
        match level {
            Some(l) if l.modes.contains(&mode) => {
                let rule = RuleSpec {
                    scope,
                    scope_id: l.scope_id,
                    mode,
                };
                self.send_decision(id, PermDecision::Always, Some(rule))
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn rejected(&mut self, id: i64, status: ReqStatus) {
        self.local.remove(&id);
        self.resolve_request(id, status);
    }

    /// Cards without a message (the request arrived as a `permission.request` frame), in ascending order of number.
    pub(crate) fn orphan_cards(&self) -> Vec<ChatItem> {
        let shown: Vec<i64> = self.messages.iter().filter_map(card_request_id).collect();
        let mut ids: Vec<_> = self
            .requests
            .keys()
            .filter(|k| !shown.contains(k))
            .copied()
            .collect();
        ids.sort_unstable();
        ids.iter()
            .filter_map(|i| self.requests.get(i))
            .map(|r| self.card_item(r))
            .collect()
    }
}
