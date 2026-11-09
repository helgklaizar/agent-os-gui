//! Gateway types: the incoming message, the intake outcome, errors and the storage port.

use serde_json::{Value, json};

use super::Contact;
use crate::{FieldError, PortFuture};

/// Limit of the message body (bytes) and of the `ext_id` length.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_EXT_ID: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }
}

/// An incoming channel message.
#[derive(Debug, Clone)]
pub struct Ingest {
    pub account_id: i64,
    pub ext_id: String,
    pub direction: Direction,
    /// Sender identifier in the channel: `@nick`, number, address.
    pub handle: String,
    pub body: String,
    pub ts: i64,
}

#[derive(Debug, PartialEq)]
pub enum GatewayError {
    Invalid(Vec<FieldError>),
    /// The account does not exist or was deleted.
    NoAccount,
    Unavailable,
}

/// Row to write into `gw_messages`.
#[derive(Debug, Clone)]
pub struct NewGwMessage {
    pub account_id: i64,
    pub ext_id: String,
    pub direction: Direction,
    pub contact: Option<Contact>,
    pub body: String,
    pub routed_session_id: Option<i64>,
    pub isolated: bool,
    pub ts: i64,
}

/// What the write returned: the row `id` and the contact `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    pub id: i64,
    pub contact_id: Option<i64>,
}

/// Gateway storage (implemented by `agentd-db`).
pub trait GatewayStore: Send + Sync + 'static {
    /// Account type (`tg`, `gmail`, ...); `None` means missing or deleted.
    fn account_type(&self, account_id: i64)
    -> PortFuture<'_, Result<Option<String>, GatewayError>>;
    /// Contact (by name, updating handles) and message in one transaction; `None` means a duplicate `ext_id`.
    fn store(&self, m: NewGwMessage) -> PortFuture<'_, Result<Option<Stored>, GatewayError>>;
}

/// Intake outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Ingested {
    pub id: Option<i64>,
    pub duplicate: bool,
    pub routed_session_id: Option<i64>,
    pub isolated: bool,
    pub contact_id: Option<i64>,
}

impl Ingested {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "duplicate": self.duplicate, "routed_session_id": self.routed_session_id,
               "isolated": self.isolated, "contact_id": self.contact_id})
    }
}
