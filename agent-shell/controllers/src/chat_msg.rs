//! Messages of `ChatController` (input from `ui` and the core layer).

use serde_json::Value;

use crate::chat_types::ReqStatus;
use crate::chat_view::ButtonKind;
use crate::permissions::{Mode, PermTable, RuleScope};
use crate::tree_types::PersonaItem;

/// Input field key: `Enter` sends, `Shift+Enter` inserts a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKey {
    Enter,
    ShiftEnter,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatMsg {
    /// The active session changed; `None` means there are no sessions.
    Select(Option<i64>),
    /// A `session.<id>` frame.
    Frame {
        session_id: i64,
        snapshot: bool,
        data: Value,
    },
    /// The input field text changed.
    DraftChanged(String),
    /// A key was pressed in the input field; `client_id` is a UUID in case of sending.
    Key {
        key: InputKey,
        client_id: String,
    },
    /// The "Send" button.
    Send {
        client_id: String,
    },
    /// The user is at the bottom of the stream or scrolling up.
    Scrolled {
        at_bottom: bool,
    },
    Personas(Vec<PersonaItem>),
    /// The "Attach" button: the view opens a file dialog.
    AttachClicked,
    /// A file was chosen; the controller does not read its content.
    AttachPicked {
        path: String,
        name: String,
        size: u64,
    },
    /// The dialog was closed without a choice.
    AttachDialogClosed,
    /// Remove an attachment from the draft by index.
    AttachRemove(usize),
    /// Permission matrix (`GET /permissions`) for choosing the level in "Always".
    Permissions(PermTable),
    /// A button of the permission request card.
    CardButton {
        request_id: i64,
        button: ButtonKind,
    },
    /// Choice of the rule level and mode in "Always".
    AlwaysPick {
        request_id: i64,
        scope: RuleScope,
        mode: Mode,
    },
    AlwaysCancel {
        request_id: i64,
    },
    /// The core answered `409`: the request was already resolved by another client, `status` is its outcome.
    DecisionRejected {
        request_id: i64,
        status: ReqStatus,
    },
    /// The decision did not arrive (network, `403`): the card returns to the question.
    DecisionFailed {
        request_id: i64,
    },
    /// Click on the persona chip: open the list.
    PersonaChipClicked,
    ClosePersonaList,
    /// Selection in the list; `None` means "same as folder" (inherit).
    PickPersona(Option<i64>),
    /// Click on the folder chip: the view opens a file dialog.
    PathChipClicked,
    /// Dialog result; `None` means cancelled.
    PickPath(Option<String>),
    /// The "Stop" button.
    Stop,
    /// The core rejected the send (`4xx`, the item was removed from the queue): the bubble is marked as not sent.
    SendFailed {
        client_id: String,
    },
}
