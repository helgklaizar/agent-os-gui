//! Controller contract and its side effects.

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::chat_types::AttachmentRef;
use crate::notif_types::EventsFilter;
use crate::permissions::{Mode, PermDecision, RuleSpec};
use crate::prefs::UiPrefs;
use crate::tool_types::{Link, ToolResult};
use crate::types::Theme;

/// A command to the core. A controller sends nothing itself: the layer above (`ui` + `core-client`) executes the effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreCommand {
    /// `PATCH /v1/settings/theme` with `If-Match`; on `412` the layer above re-reads the value and sends it to the controller.
    PatchTheme(Theme),
    /// `POST /v1/logout`, then exit the shell (confirmed in the dialog).
    Logout,
    /// `POST /v1/sessions`.
    CreateSession {
        name: String,
        folder_id: Option<i64>,
        persona_id: Option<i64>,
    },
    /// `POST /v1/folders`.
    CreateFolder {
        name: String,
        parent_id: Option<i64>,
        persona_id: Option<i64>,
    },
    /// `POST /v1/sessions/{id}/threads`.
    CreateThread {
        parent_id: i64,
        name: String,
    },
    /// `PATCH /v1/sessions/{id}` with a single `folder_id`: the name is untouched.
    MoveSession {
        id: i64,
        folder_id: Option<i64>,
    },
    /// `PATCH /v1/sessions/{id}` with a single `name`: the folder is untouched.
    RenameSession {
        id: i64,
        name: String,
    },
    /// `PATCH /v1/folders/{id}` with a single `name`.
    RenameFolder {
        id: i64,
        name: String,
    },
    DeleteSession {
        id: i64,
    },
    DeleteFolder {
        id: i64,
    },
    /// `POST /v1/sessions/{id}/read`.
    MarkRead {
        id: i64,
    },
    /// `POST /v1/sessions/{id}/messages`; `client_id` provides idempotency, the layer above uploads attachments.
    SendMessage {
        session_id: i64,
        client_id: String,
        body_md: String,
        attachments: Vec<AttachmentRef>,
    },
    /// `POST /v1/sessions/{id}/stop`: stop the agent's current reply.
    StopStream {
        session_id: i64,
    },
    /// `PATCH /v1/sessions/{id}` with a single `persona_id` (`None` means inherit).
    SetSessionPersona {
        session_id: i64,
        persona_id: Option<i64>,
    },
    /// `PATCH /v1/sessions/{id}` with a single `path`.
    SetSessionPath {
        session_id: i64,
        path: String,
    },
    /// `GET /v1/personas` for the chip list; the answer goes to `ChatMsg::Personas`.
    ListPersonas,
    /// `POST /v1/permission-requests/{id}/decision`; `rule` is required only for `Always` (docs/reference/api.md, section 5.4).
    DecidePermission {
        request_id: i64,
        decision: PermDecision,
        rule: Option<RuleSpec>,
    },
    /// A delta not matching `offset` or lost: the layer above does `unsubscribe` + `subscribe` for topic `session.<id>` (docs/reference/api.md, section 4.4).
    ResyncSession {
        session_id: i64,
    },
    /// `POST /v1/events/{id}/read`.
    MarkEventRead {
        id: i64,
    },
    /// `POST /v1/events/read` with a filter (empty means all unread).
    MarkEventsRead(EventsFilter),
    /// `GET /v1/events/subtypes`; the answer goes to `NotifMsg::Subtypes`.
    FetchEventSubtypes,
    /// Counters diverged from the core (something was read that is not in the feed): the layer above does `unsubscribe` + `subscribe` for topic `events`.
    ResyncEvents,
    /// Read the core version (`GET /v1/health`) for the About dialog.
    FetchAbout,
    /// `GET /v1/browser/quicklinks`; the answer goes to `BrowserMsg::Quick`.
    FetchQuickLinks,
    /// `GET /v1/sessions/{id}/bookmarks`; the answer goes to `BrowserMsg::Marks`.
    FetchBookmarks {
        session_id: i64,
    },
    /// `PUT /v1/browser/quicklinks` with `If-Match: <rev>`; on `412` the layer above re-reads the list.
    PutQuickLinks {
        rev: i64,
        items: Vec<Link>,
    },
    /// `PUT /v1/sessions/{id}/bookmarks` with `If-Match: <rev>`.
    PutBookmarks {
        session_id: i64,
        rev: i64,
        items: Vec<Link>,
    },
    /// `POST /v1/sessions/{id}/tool-results`: result of an agent command to the browser. Terminal output does not go here.
    PostToolResult {
        session_id: i64,
        result: ToolResult,
    },
    /// `PATCH /v1/sessions/{id}` with `cfg` made of the given fields.
    PatchSessionCfg {
        session_id: i64,
        cfg: CfgPatch,
    },
    /// `GET /v1/permissions`; the answer goes to `SessionSettingsMsg::Perms`.
    FetchPermissions,
    /// `PUT /v1/permissions/session/{id}/{action}`: tightening only, the core rejects loosening with `403`.
    PutSessionPerm {
        session_id: i64,
        action: String,
        mode: Mode,
    },
    /// `POST /v1/sessions/{id}/clear` (only after confirmation).
    ClearSession {
        session_id: i64,
    },
    /// `POST /v1/sessions/{id}/export`.
    ExportSession {
        session_id: i64,
    },
    /// `GET /v1/sessions/{id}/tasks`; the answer goes to `SessionSettingsMsg::Tasks`.
    FetchTasks {
        session_id: i64,
    },
    /// `POST /v1/sessions/{id}/tasks`.
    CreateTask {
        session_id: i64,
        kind: String,
        title: String,
        spec: String,
        cron: Option<String>,
    },
    /// `PATCH /v1/tasks/{id}` with a single `state` (pause and resume).
    SetTaskState {
        task_id: i64,
        state: String,
    },
    /// `POST /v1/tasks/{id}/run`.
    RunTask {
        task_id: i64,
    },
    /// `GET /v1/tasks/{id}/runs`; the answer goes to `SessionSettingsMsg::Runs`.
    FetchTaskRuns {
        task_id: i64,
    },
    /// A Settings dialog command; the answer goes to `SettingsMsg::Reply`.
    Settings(crate::settings_types::Cmd),
    /// `GET /v1/search?q=&limit=` ; the layer above parses the answer with `hits::parse_hits` and sends it to `SearchController`.
    Search {
        query: String,
        limit: u32,
    },
}

/// Session `cfg` fields (`None` means leave unchanged): `ctx` is `parent` or `none`, `model` is `account_id` (`Some(None)` means "same as persona"),
/// `notif` is `all`, `important` or `off`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CfgPatch {
    pub ctx: Option<String>,
    pub model: Option<Option<i64>>,
    pub mem: Option<bool>,
    pub auto: Option<bool>,
    pub notif: Option<String>,
}

/// A side effect of a controller step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Publish an event to the bus.
    Publish(BusEvent),
    /// A command to the core client.
    Command(CoreCommand),
    /// Write part of the settings to `ui.json` (`Some` fields are merged into the file).
    Persist(UiPrefs),
}

/// A state machine without GTK and without async.
pub trait Controller {
    type Msg;
    type View: Clone + PartialEq;

    /// Pure state function: the input is a message and the current time (clocks are controllable in tests).
    fn update(&mut self, msg: Self::Msg, now: Instant) -> Vec<Effect>;
    fn view(&self) -> Self::View;
}
