//! `ChatController`: messages of the active session, input, stream, chips, permission cards.

use crate::time::Instant;
use std::collections::HashMap;

use serde_json::Value;

use crate::chat_bubbles::item;
use crate::chat_cards::card_request_id;
use crate::chat_types::{
    AttachmentRef, Message, PermRequest, SessionFrame, SessionInfo, parse_session_frame,
};
use crate::chat_view::{AttachPreview, ChatItem, ChatView};
use crate::controller::{Controller, Effect};
use crate::i18n::tr;
use crate::permissions::{PermDecision, PermTable};
use crate::tree_types::PersonaItem;

pub use crate::chat_msg::{ChatMsg, InputKey};

/// A message that was sent but not yet confirmed by the core.
pub(crate) struct Pending {
    pub(crate) session_id: i64,
    pub(crate) client_id: String,
    pub(crate) body: String,
    pub(crate) attachments: Vec<AttachPreview>,
    /// The core rejected the send.
    pub(crate) failed: bool,
}

pub struct ChatController {
    pub(crate) session: Option<i64>,
    pub(crate) info: SessionInfo,
    pub(crate) personas: Vec<PersonaItem>,
    pub(crate) messages: Vec<Message>,
    pub(crate) pending: Vec<Pending>,
    pub(crate) drafts: HashMap<i64, String>,
    pub(crate) at_bottom: bool,
    pub(crate) scroll_seq: u64,
    pub(crate) utc_offset: i32,
    pub(crate) typing_on: bool,
    /// Attachments chosen but not yet sent, per session.
    pub(crate) staged: HashMap<i64, Vec<AttachmentRef>>,
    pub(crate) want_attach_dialog: bool,
    pub(crate) requests: HashMap<i64, PermRequest>,
    /// What the user has already answered to a request: a second answer is not sent.
    pub(crate) local: HashMap<i64, PermDecision>,
    pub(crate) always_open: Option<i64>,
    pub(crate) perms: PermTable,
    pub(crate) persona_list_open: bool,
    pub(crate) want_path_dialog: bool,
    /// A snapshot was requested after an `offset` mismatch; we do not ask again until it arrives.
    pub(crate) resync: bool,
}

impl Default for ChatController {
    fn default() -> Self {
        ChatController {
            session: None,
            info: SessionInfo::default(),
            personas: Vec::new(),
            messages: Vec::new(),
            pending: Vec::new(),
            drafts: HashMap::new(),
            at_bottom: true,
            scroll_seq: 0,
            utc_offset: 0,
            typing_on: false,
            staged: HashMap::new(),
            want_attach_dialog: false,
            requests: HashMap::new(),
            local: HashMap::new(),
            always_open: None,
            perms: PermTable::default(),
            persona_list_open: false,
            want_path_dialog: false,
            resync: false,
        }
    }
}

impl ChatController {
    pub fn new() -> ChatController {
        ChatController::default()
    }

    /// Time zone offset for the time label (seconds from UTC).
    pub fn with_utc_offset(mut self, secs: i32) -> ChatController {
        self.utc_offset = secs;
        self
    }

    pub(crate) fn persona(&self) -> Option<&PersonaItem> {
        let id = self.info.persona_id?;
        self.personas.iter().find(|p| p.id == id)
    }

    pub(crate) fn draft(&self) -> &str {
        self.session
            .and_then(|s| self.drafts.get(&s))
            .map_or("", String::as_str)
    }

    pub(crate) fn follow(&mut self) {
        if self.at_bottom {
            self.scroll_seq += 1;
        }
    }

    fn select(&mut self, id: Option<i64>) {
        if id == self.session {
            return;
        }
        self.session = id;
        self.info = SessionInfo {
            id: id.unwrap_or_default(),
            ..SessionInfo::default()
        };
        self.messages.clear();
        self.requests.clear();
        self.always_open = None;
        self.want_attach_dialog = false;
        self.typing_on = false;
        self.persona_list_open = false;
        self.want_path_dialog = false;
        self.resync = false;
        self.at_bottom = true;
        self.scroll_seq += 1;
    }

    pub(crate) fn upsert(&mut self, m: Message) {
        self.register_card(&m);
        self.pending
            .retain(|p| m.client_id.as_deref() != Some(&p.client_id));
        match self.messages.binary_search_by_key(&m.id, |x| x.id) {
            Ok(i) => self.messages[i] = m,
            Err(i) => self.messages.insert(i, m),
        }
    }

    fn on_frame(&mut self, snapshot: bool, data: &Value) -> Vec<Effect> {
        let Some(frame) = parse_session_frame(snapshot, data) else {
            return Vec::new();
        };
        match frame {
            SessionFrame::Snapshot {
                session, messages, ..
            } => {
                if let Some(s) = session {
                    self.info = s;
                }
                self.messages.clear();
                for m in messages {
                    self.upsert(m);
                }
                self.at_bottom = true;
                self.scroll_seq += 1;
                self.resync = false;
                // A `typing:false` frame may have been lost along with the connection: after a snapshot the indicator is derived from the messages.
                self.typing_on = false;
            }
            SessionFrame::Created(m) => {
                self.upsert(m);
                self.follow();
            }
            SessionFrame::Updated(s) => self.info = s,
            SessionFrame::Delta {
                message_id,
                offset,
                text,
            } => {
                return self.on_delta(message_id, offset, &text);
            }
            SessionFrame::Done {
                message_id,
                status,
                final_len,
            } => {
                return self.on_done(message_id, status, final_len);
            }
            SessionFrame::Typing(on) => self.typing_on = on,
            SessionFrame::Request(r) => self.register_request(r),
            SessionFrame::Resolved { request_id, status } => {
                self.resolve_request(request_id, status)
            }
        }
        Vec::new()
    }
}

impl Controller for ChatController {
    type Msg = ChatMsg;
    type View = ChatView;

    fn update(&mut self, msg: ChatMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            ChatMsg::Select(id) => self.select(id),
            ChatMsg::Frame {
                session_id,
                snapshot,
                data,
            } => {
                if self.session == Some(session_id) {
                    return self.on_frame(snapshot, &data);
                }
            }
            ChatMsg::DraftChanged(text) => {
                if let Some(s) = self.session {
                    self.drafts.insert(s, text);
                }
            }
            ChatMsg::Key {
                key: InputKey::Enter,
                client_id,
            }
            | ChatMsg::Send { client_id } => {
                return self.submit(client_id);
            }
            ChatMsg::Key {
                key: InputKey::ShiftEnter,
                ..
            } => {
                if let Some(s) = self.session {
                    self.drafts.entry(s).or_default().push('\n');
                }
            }
            ChatMsg::Scrolled { at_bottom } => self.at_bottom = at_bottom,
            ChatMsg::Personas(p) => self.personas = p,
            ChatMsg::Stop => return self.stop(),
            ChatMsg::SendFailed { client_id } => {
                if let Some(p) = self.pending.iter_mut().find(|p| p.client_id == client_id) {
                    p.failed = true;
                }
            }
            ChatMsg::AttachClicked => self.want_attach_dialog = self.session.is_some(),
            ChatMsg::AttachPicked { path, name, size } => {
                return self.attach_picked(path, name, size);
            }
            ChatMsg::AttachDialogClosed => self.want_attach_dialog = false,
            ChatMsg::AttachRemove(i) => self.attach_remove(i),
            ChatMsg::Permissions(t) => self.perms = t,
            ChatMsg::CardButton { request_id, button } => return self.press(request_id, button),
            ChatMsg::AlwaysPick {
                request_id,
                scope,
                mode,
            } => {
                return self.always_pick(request_id, scope, mode);
            }
            ChatMsg::AlwaysCancel { request_id } => {
                if self.always_open == Some(request_id) {
                    self.always_open = None;
                }
            }
            ChatMsg::DecisionRejected { request_id, status } => self.rejected(request_id, status),
            ChatMsg::DecisionFailed { request_id } => {
                self.local.remove(&request_id);
            }
            ChatMsg::PersonaChipClicked => return self.open_persona_list(),
            ChatMsg::ClosePersonaList => self.persona_list_open = false,
            ChatMsg::PickPersona(id) => return self.pick_persona(id),
            ChatMsg::PathChipClicked => self.want_path_dialog = self.session.is_some(),
            ChatMsg::PickPath(path) => return self.pick_path(path),
        }
        Vec::new()
    }

    fn view(&self) -> ChatView {
        let mut items: Vec<ChatItem> = self
            .messages
            .iter()
            .filter_map(|m| match card_request_id(m) {
                Some(id) => self.requests.get(&id).map(|r| self.card_item(r)),
                None => Some(item(m, self.persona(), self.utc_offset)),
            })
            .collect();
        items.extend(self.orphan_cards());
        items.extend(
            self.pending
                .iter()
                .filter(|p| Some(p.session_id) == self.session)
                .map(|p| ChatItem::Bubble(self.pending_bubble(p))),
        );
        ChatView {
            session_id: self.session,
            empty: self.session.is_none().then(|| tr("chat-empty")),
            items,
            draft: self.draft().to_string(),
            can_send: self.session.is_some()
                && (!self.draft().trim().is_empty() || !self.staged().is_empty()),
            scroll_seq: self.scroll_seq,
            typing: (self.typing_on || self.is_streaming()).then(|| tr("chat-typing")),
            can_stop: self.is_streaming(),
            chips: self.chips(),
            persona_choices: self.persona_choices(),
            want_path_dialog: self.want_path_dialog,
            attachments: self.staged_previews(),
            want_attach_dialog: self.want_attach_dialog,
        }
    }
}
