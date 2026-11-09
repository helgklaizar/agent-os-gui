//! `agentd` domain: types without I/O. Currently only token kinds and scopes.

use serde::{Deserialize, Serialize};

mod actions;
mod audit;
mod auth;
mod browser;
mod channels;
mod chat;
mod clock;
mod events;
mod export;
mod files;
mod gate;
mod gateway;
mod indicators;
mod links;
mod logout;
mod mcp;
mod memory;
mod memory_source;
mod merge;
mod model;
mod modules;
mod notify;
mod observer;
mod perms;
mod ports;
mod quota;
mod requests;
mod respond;
mod rules;
mod search;
mod secret;
mod secrets;
mod settings_json_schema;
mod settings_models;
mod settings_schema;
mod shell;
mod shell_token;
mod sysmetrics;
mod tasks;
mod tools_catalog;
pub mod tree;
mod triggers;
mod tunnels;
mod watch;
mod weather;

pub use actions::{
    ActionCall, ActionExecutor, ConfigExecutor, ExecError, Executors, Unavailable, action_for_tool,
    target_of,
};
pub use audit::{
    ActionQuery, ActionRow, Actor, Audit, AuditStore, Decision, Finding, MASK, NewAction,
    PermError, PermResult, mask_args, mask_text,
};
pub use auth::{AuthFailure, AuthFuture, Authenticator, Principal};
pub use browser::{
    BROWSE_TIMEOUT_SECS, BrowseExecutor, BrowserBridge, BrowserOutcome, MAX_OUTPUT_BYTES,
    SubmitError, ToolResult, parse_tool_result,
};
pub use channels::{
    Capability, ChannelAccount, ChannelControl, ChannelError, ChannelProvider, ChannelRegistry,
    ChannelSend, Channels, ConnectKind, Flow, FlowStatus, Health, Incoming, Outcome as SendOutcome,
};
pub use chat::{
    AccountRow, Attachment, ChatContext, ChatError, ChatResult, ChatStore, HistoryItem,
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, MAX_BODY_BYTES, MAX_PAGE, Posted, UserMessage,
    parse_user_message, valid_client_id,
};
#[cfg(feature = "test-support")]
pub use clock::ManualClock;
pub use clock::{Clock, SystemClock, manual_clock};
pub use events::{
    DEFAULT_LIMIT, Event, EventError, EventFilter, EventLevel, EventResult, EventStore, Events,
    KINDS, MAX_LIMIT, MAX_SUB_CHARS, MAX_TEXT_CHARS, NewEvent, ReadSelector, SUBTYPES, Subtype,
    parse_filter, parse_read, subtypes_json, tab_of,
};
pub use export::{ExportError, ExportInfo, SessionExporter};
pub use files::{
    FileError, FileInfo, FileKind, FilePath, FileStore, MAX_FILE_BYTES, MISSING_REV, normalize,
};
pub use gate::{Gate, GateInput, PolicyGate, Subject};
pub use gateway::{
    Contact, Direction, Gateway, GatewayError, GatewayStore, IncomingHook, IncomingMsg, Ingest,
    Ingested, MAX_BODY_BYTES as MAX_GW_BODY_BYTES, NewGwMessage, Stored, channel_of, find_mismatch,
    match_contact, route_for, validate_connections,
};
pub use indicators::{
    AgentdStatus, AiUsage, BACKUP_BAD_SECS, BACKUP_WARN_SECS, DEFAULT_INDICATORS, IndicatorEngine,
    IndicatorError, IndicatorSource, IndicatorState, IndicatorStore, Link, Observed,
    ObserverHealth, Reading, SysIndicator, SystemIndicator, SystemProbe, Unread, default_items,
    grade_backup, grade_battery, grade_net, grade_vpn, valid_id,
};
pub use links::{MAX_LINKS, QUICKLINKS_KEY, bookmarks_key, default_quicklinks, validate_links};
pub use logout::SessionCloser;
pub use mcp::{
    CallResult as McpCallResult, DEFAULT_RISK as MCP_DEFAULT_RISK, Mcp, McpConnector, McpError,
    McpNotice, McpNoticeHook, McpSession, McpTool, RemoteTool, ServerSpec as McpServerSpec,
    ServerStatus as McpStatus, Transport as McpTransport, tool_name as mcp_tool_name,
};
pub use memory::{
    CONTEXT_FACTS, MAX_FACT_BYTES, MAX_FACTS_PER_MINUTE, MemoryError, MemoryExecutor, MemoryStore,
};
pub use memory_source::{MEMORY_MAX_FAILS, MEMORY_PAUSE, MEMORY_TIMEOUT, McpMemorySource};
pub use merge::merge_patch;
pub use model::{
    LimitWindow, ModelAccount, ModelProvider, ModelRequest, ProviderError, ProviderEvent,
    ProviderKind, ProviderRegistry, RateLimit, ToolCall, ToolSpec, Turn, TurnRole, Usage,
    pick_account,
};
pub use modules::{
    FetchError, MAX_DOWNLOAD_BYTES, Module, ModuleError, ModuleFetcher, ModuleStore, PLACEMENTS,
    check_install_url, check_install_url_allowing, is_internal as is_internal_ip,
    validate_manifest,
};
pub use notify::{
    Limiter as NotifyLimiter, Notification, PER_WINDOW as NOTIFY_PER_WINDOW,
    SOURCE as NOTIFY_SOURCE, WINDOW_SECS as NOTIFY_WINDOW_SECS,
};
pub use observer::{Hit, Observer, ObserverStatus, RuleError, RuleObserver};
pub use perms::{
    Action, Level, Mode, Rule, RuleSet, effective_mode, loosening_levels, would_loosen,
};
pub use ports::{
    ExternalTokenRequest, FieldError, IssuedExternal, IssuedModuleToken, PortFuture, RevokeOutcome,
    SettingsDoc, SettingsError, SettingsStore, StatusSource, TokenAdmin, TokenAdminError,
    TokenInfo,
};
pub use quota::{
    Price, QuotaStore, UsageRecord, WINDOW_SECS, eta_at, rate_of, tightest, used_fraction,
};
pub use requests::{
    DEFAULT_TTL_SECS, DecideError, DecideInput, DecisionKind, Outcome, PermissionRequest,
    PermissionService, RequestSpec, RuleInput, Status,
};
pub use respond::{ChatConfig, ChatService, RunAs, TurnEnd};
pub use rules::{PermEngine, PutError, RuleStore};
pub use search::{
    DEFAULT_LIMIT as SEARCH_DEFAULT_LIMIT, MAX_LIMIT as SEARCH_MAX_LIMIT, SearchError, SearchHit,
    SearchStore, parse_query as parse_search_query,
};
pub use secret::Secret;
pub use secrets::{
    MAX_SECRET_BYTES, SecretError, SecretInfo, SecretStore, account_of_ref, valid_secret_ref,
};
pub use settings_schema::{
    MAX_DOC_BYTES, PAGES, default_doc, is_known_page, is_read_only, secret_fields,
    validate as validate_settings, validate_patch as validate_settings_patch,
};
pub use shell::{
    DEFAULT_TIMEOUT_SECS, MAX_TERMINAL_OUTPUT, MAX_TIMEOUT_SECS, RunDone, RunEvent, RunSpec,
    RunnerError, ShellExecutor, ShellRunner,
};
pub use shell_token::{AgentToken, AgentTokens};
pub use sysmetrics::SysSnapshot;
pub use tasks::{
    CronError, DEFAULT_MAX_PARALLEL, DEFAULT_TZ, MISSED_GRACE_SECS, NewTask, RUNS_DEFAULT_LIMIT,
    RUNS_MAX_LIMIT, Task, TaskCreate, TaskEdit, TaskError, TaskFilter, TaskResult, TaskRun,
    TaskStore, TaskUpdate, Tasks, cron_check, cron_next, parse_create, parse_edit,
    parse_filter as parse_task_filter,
};
pub use tools_catalog::builtin_tools;
pub use tree::{
    AccountInput, FolderInput, Notifier, PersonaInput, SecretEraser, SessionFilter, SessionInput,
    TreeError, TreeResult, TreeStore,
};
pub use triggers::{
    STORM_LIMIT, STORM_WINDOW_SECS, Trigger, TriggerCreate, TriggerEdit, TriggerError,
    TriggerResult, TriggerStore, Triggers, parse_create as parse_trigger_create,
    parse_edit as parse_trigger_edit,
};
pub use tunnels::{
    TUNNEL_TYPES, Tunnel, TunnelError, TunnelStatuses, TunnelStore, looks_secret,
    parse_create as parse_tunnel_create, parse_patch as parse_tunnel_patch,
};
pub use watch::{
    AccountPicker, Class as WatchClass, Classifier, ClassifyError,
    DEFAULT_MODEL as WATCH_DEFAULT_MODEL, DEFAULT_TIMEOUT as WATCH_DEFAULT_TIMEOUT,
    MAX_SUBJECT_CHARS as WATCH_MAX_SUBJECT_CHARS, ModelClassifier, ModelObserver, RemoteObserver,
    StoredAccounts, Subject as WatchSubject, Verdict as WatchVerdict,
    parse_verdict as parse_watch_verdict,
};
pub use weather::{WeatherNow, WeatherSnapshot, location_of};

/// Token scope; strictness order `read < chat < decide < admin` (`docs/reference/api.md` 1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Read,
    Chat,
    /// `chat` + a one-off decision on a permission request (kind `web` only).
    Decide,
    Admin,
}

impl Scope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Chat => "chat",
            Scope::Decide => "decide",
            Scope::Admin => "admin",
        }
    }

    /// Only values from the `CHECK` of the `api_token` table; case matters.
    #[must_use]
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "read" => Some(Scope::Read),
            "chat" => Some(Scope::Chat),
            "decide" => Some(Scope::Decide),
            "admin" => Some(Scope::Admin),
            _ => None,
        }
    }

    /// Whether a token with this scope meets the route requirement.
    #[must_use]
    pub fn satisfies(self, required: Scope) -> bool {
        self >= required
    }
}

/// Token kind (`api_token.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Gui,
    External,
    Test,
    Agent,
    Module,
    /// Browser client: `chat` or `decide`, lifetime up to 30 days, not `admin`.
    Web,
}

impl TokenKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Gui => "gui",
            TokenKind::External => "external",
            TokenKind::Test => "test",
            TokenKind::Agent => "agent",
            TokenKind::Module => "module",
            TokenKind::Web => "web",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<TokenKind> {
        match s {
            "gui" => Some(TokenKind::Gui),
            "external" => Some(TokenKind::External),
            "test" => Some(TokenKind::Test),
            "agent" => Some(TokenKind::Agent),
            "module" => Some(TokenKind::Module),
            "web" => Some(TokenKind::Web),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_order_and_roundtrip() {
        assert!(Scope::Admin.satisfies(Scope::Chat));
        assert!(Scope::Chat.satisfies(Scope::Chat));
        assert!(!Scope::Read.satisfies(Scope::Chat));
        for s in [Scope::Read, Scope::Chat, Scope::Admin] {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
        assert_eq!(Scope::parse("Admin"), None);
        assert_eq!(Scope::parse(""), None);
    }

    #[test]
    fn kind_roundtrip_rejects_unknown() {
        for k in [
            TokenKind::Gui,
            TokenKind::External,
            TokenKind::Test,
            TokenKind::Agent,
            TokenKind::Module,
        ] {
            assert_eq!(TokenKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(TokenKind::parse("service"), None);
    }
}
