//! Chat view for `ui`: flat data without Pango markup (escaping is done by the view).

use crate::markdown::Block;
use crate::permissions::{Mode, RuleScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// Queued; the core has not confirmed yet.
    Pending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachPreview {
    pub name: String,
    pub size_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    pub id: Option<i64>,
    pub client_id: Option<String>,
    pub mine: bool,
    pub author_line: String,
    pub time: String,
    pub blocks: Vec<Block>,
    pub streaming: bool,
    /// "Reply interrupted" and the like.
    pub note: Option<String>,
    pub delivery: Delivery,
    pub attachments: Vec<AttachPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCard {
    pub message_id: i64,
    pub title: String,
    /// Reply text accumulated before the error (as is, without markup).
    pub text: String,
}

/// Card button: what was pressed and its label from `en.ftl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Allow,
    Deny,
    Always,
    Confirm,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardButton {
    pub kind: ButtonKind,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    Pending,
    Allowed,
    Denied,
    Always,
    Expired,
}

/// Level of the "Always" rule: persona or global (there is no session level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelOption {
    pub scope: RuleScope,
    pub scope_id: i64,
    pub label: String,
    /// Modes no weaker than the higher-level rule.
    pub modes: Vec<Mode>,
    pub default_mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlwaysChooser {
    pub levels: Vec<LevelOption>,
}

/// Card of an `ask` permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskCard {
    pub request_id: i64,
    pub title: String,
    pub target: String,
    pub detail: Option<String>,
    pub state: CardState,
    /// "allowed" / "denied" / "always" / "expired" after the decision.
    pub state_label: Option<String>,
    pub buttons: Vec<CardButton>,
    pub always: Option<AlwaysChooser>,
}

/// What the observer detected (`observer_findings.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverKind {
    Injection,
    Phishing,
    DangerousCmd,
    Module,
    PermissionLoosen,
    Other,
}

/// Observer verdict (`observer_findings.verdict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Blocked,
    Asked,
    Allowed,
}

/// Observer confirmation: red frame, what was detected, what happens to the action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverCard {
    pub request_id: i64,
    pub kind: ObserverKind,
    pub kind_label: String,
    pub target: String,
    pub detail: Option<String>,
    pub verdict: Verdict,
    pub verdict_label: String,
    pub red_border: bool,
    /// The action is not performed until the user confirms.
    pub awaiting: bool,
    pub state: CardState,
    pub state_label: Option<String>,
    /// "Confirm" / "Reject" for the `asked` verdict.
    pub buttons: Vec<CardButton>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    Observer(ObserverCard),
    Bubble(Bubble),
    Error(ErrorCard),
    Ask(AskCard),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chips {
    pub persona_label: String,
    pub persona_color: Option<String>,
    pub path_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaChoice {
    /// `None` means "same as folder".
    pub id: Option<i64>,
    pub name: String,
    pub color: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatView {
    pub session_id: Option<i64>,
    /// "Create a session on the left" when there are no sessions.
    pub empty: Option<String>,
    pub items: Vec<ChatItem>,
    pub draft: String,
    pub can_send: bool,
    /// Grows when the view must scroll the stream down.
    pub scroll_seq: u64,
    /// "the agent is typing…" while streaming.
    pub typing: Option<String>,
    pub can_stop: bool,
    pub chips: Option<Chips>,
    /// Persona list while it is open via a chip click (from `GET /personas`, not `prompt()`).
    pub persona_choices: Option<Vec<PersonaChoice>>,
    /// The view must open the folder-selection file dialog.
    pub want_path_dialog: bool,
    /// Draft attachments: name and size, the content is not shown.
    pub attachments: Vec<AttachPreview>,
    pub want_attach_dialog: bool,
}
