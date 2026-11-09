//! GUI <-> agentd contract types ([api.md](../../../../docs/reference/api.md), `openapi-draft.yaml`).
//!
//! The client must ignore unknown fields and unknown `type` values (api.md section 2), so there is no `deny_unknown_fields` anywhere.

use std::fmt;

use serde::{Deserialize, Serialize};

mod error;
mod frames;

pub use error::{ApiError, ErrorCode};
pub use frames::{ClientOp, Frame, FrameKind, ServerMessage, parse_server_message};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Read,
    Chat,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Gui,
    External,
    Test,
    Agent,
    Module,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
    Starting,
    Degraded,
    Migrating,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Migration {
    pub status: String,
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub progress: Option<f64>,
}

/// `GET /v1/health` (api.md section 2.1).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Health {
    pub status: HealthStatus,
    pub version: String,
    pub api: ApiVersion,
    #[serde(default)]
    pub min_client: Option<String>,
    #[serde(default)]
    pub min_gui: Option<String>,
    #[serde(default)]
    pub max_gui: Option<String>,
    #[serde(default)]
    pub user_version: Option<i64>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub migration: Option<Migration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Auto,
    Dark,
    Light,
}

/// `GET`/`PATCH /v1/settings/theme`; `rev` goes into `If-Match`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ThemePage {
    pub rev: i64,
    pub theme: Theme,
}

/// Token description without the value (`Token` in OpenAPI).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TokenInfo {
    pub id: i64,
    pub name: String,
    pub kind: TokenKind,
    pub scope: Scope,
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
    #[serde(default)]
    pub created_by: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub last_used: Option<i64>,
    pub revoked: bool,
}

/// Body of `POST /v1/tokens`. `kind` and `allowlist` are a `fake-agentd` extension for checking the lifecycle:
/// the real core issues only `external`, and issues agent and module tokens itself (api.md section 1.1).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TokenCreate {
    pub name: String,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<TokenKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<Vec<String>>,
}

/// Response of `POST /v1/tokens`: the value is shown once, hidden in `Debug`.
#[derive(Clone, PartialEq, Deserialize)]
pub struct TokenIssued {
    #[serde(flatten)]
    pub info: TokenInfo,
    pub token: String,
}

impl fmt::Debug for TokenIssued {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenIssued")
            .field("info", &self.info)
            .field("token", &"<hidden>")
            .finish()
    }
}

/// `bar.sys` (api.md section 4.4): percentages and temperature; `null` means no data.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BarSys {
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub gpu: Option<f64>,
    #[serde(default)]
    pub ram: Option<f64>,
    #[serde(default)]
    pub disk: Option<f64>,
    #[serde(default)]
    pub temp: Option<f64>,
}

/// Balance between accounts on the `personas` page (`balance.fallback`: `local`, `other` or `wait`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Balance {
    #[serde(default)]
    pub fallback: Option<String>,
}

/// `GET /v1/settings/personas` as far as the bar needs: `rev` and balance (the page schema is defined by the core, api.md section 5).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PersonasPage {
    pub rev: i64,
    #[serde(default)]
    pub balance: Option<Balance>,
}

/// Service command `POST /v1/_test/echo` (`fake-agentd` only, not part of the contract).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EchoRequest {
    pub client_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EchoResponse {
    pub client_id: String,
    pub text: String,
    /// Sequence number of the accepted message on the mock side.
    pub ordinal: u64,
    /// `true` if this is a repeat of an already accepted `client_id`.
    pub duplicate: bool,
}
