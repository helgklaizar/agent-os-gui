//! Channel providers: trait, send outcome and relay. Account connection is in `service`, the outgoing
//! `send` action in `send`. An unknown send outcome is not retried automatically: risk of a duplicate.

mod control;
mod relay;
mod send;
mod service;

use serde_json::{Value, json};

use crate::PortFuture;

pub use control::ChannelControl;
pub use relay::ChannelRegistry;
pub use send::ChannelSend;
pub use service::{ChannelError, Channels};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Send,
    Receive,
}

/// Connection kind (`ConnectFlow.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectKind {
    Oauth,
    Qr,
    Token,
}

impl ConnectKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectKind::Oauth => "oauth",
            ConnectKind::Qr => "qr",
            ConnectKind::Token => "token",
        }
    }
}

/// Connection flow status (`ConnectFlow.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStatus {
    Pending,
    WaitingUser,
    Connected,
    Failed,
    Expired,
}

impl FlowStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FlowStatus::Pending => "pending",
            FlowStatus::WaitingUser => "waiting_user",
            FlowStatus::Connected => "connected",
            FlowStatus::Failed => "failed",
            FlowStatus::Expired => "expired",
        }
    }
}

/// Connection flow: what to show the user and which step they are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    pub kind: ConnectKind,
    pub status: FlowStatus,
    pub url: Option<String>,
    pub qr_data: Option<String>,
    pub expires_at: Option<i64>,
}

impl Flow {
    #[must_use]
    pub fn new(kind: ConnectKind, status: FlowStatus) -> Flow {
        Flow {
            kind,
            status,
            url: None,
            qr_data: None,
            expires_at: None,
        }
    }

    /// `ConnectFlow` body (`docs/reference/api.md` 6).
    #[must_use]
    pub fn to_json(&self, account_id: i64) -> Value {
        json!({"account_id": account_id, "kind": self.kind.as_str(), "status": self.status.as_str(),
               "url": self.url, "qr_data": self.qr_data, "expires_at": self.expires_at})
    }

    /// Account status (`accounts.status`): `qr` — the number is unlinked, a code must be scanned.
    #[must_use]
    pub fn account_status(&self) -> &'static str {
        match (self.status, self.kind) {
            (FlowStatus::Connected, _) => "connected",
            (FlowStatus::Failed | FlowStatus::Expired, _) => "error",
            (FlowStatus::WaitingUser | FlowStatus::Pending, ConnectKind::Qr) => "qr",
            _ => "disconnected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    Degraded,
    Down,
}

/// Send outcome. `Unknown` — no reply or it is unreadable: the delivery may have happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered { ext_id: Option<String> },
    Rejected(&'static str),
    Unknown,
}

/// Channel account for the provider; the provider reads the secret itself by the reference `account.<id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAccount {
    pub id: i64,
    pub kind: String,
}

/// Incoming message received by polling the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub ext_id: String,
    pub handle: String,
    pub body: String,
}

pub trait ChannelProvider: Send + Sync + 'static {
    /// Account type served by the provider (`tg`, `scripted-channel`).
    fn kind(&self) -> &'static str;
    fn capabilities(&self) -> &'static [Capability];
    fn connect_kind(&self) -> ConnectKind;
    /// Current connection state: checks the token or code; a repeated call is a refresh.
    fn connect<'a>(&'a self, account: &'a ChannelAccount) -> PortFuture<'a, Flow>;
    fn disconnect<'a>(&'a self, _account: &'a ChannelAccount) -> PortFuture<'a, ()> {
        Box::pin(async {})
    }
    fn health<'a>(&'a self, account: &'a ChannelAccount) -> PortFuture<'a, Health>;
    /// Send; `key` is the idempotency key, the provider does not retry.
    fn send<'a>(
        &'a self,
        account: &'a ChannelAccount,
        to: &'a str,
        text: &'a str,
        key: &'a str,
    ) -> PortFuture<'a, Outcome>;
    /// New incoming messages since the last poll.
    fn poll<'a>(&'a self, account: &'a ChannelAccount) -> PortFuture<'a, Vec<Incoming>>;
}
