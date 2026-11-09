//! Shared `fake-agentd` types: frames, channels, request records, startup error.

/// WS frame type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Snapshot,
    Delta,
    Event,
}

/// Request channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Unix,
    Tcp,
}

/// Record of an accepted request: no token and no body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    pub status: u16,
    pub request_id: String,
    pub transport: Transport,
    /// Client uid from `SO_PEERCRED` (unix socket only).
    pub peer_uid: Option<u32>,
}

/// Accepted service command `echo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoRecord {
    pub client_id: String,
    pub text: String,
    pub ordinal: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    #[error("I/O: {0}")]
    Io(String),
    #[error("{0}")]
    Config(String),
}

pub(crate) fn io(e: std::io::Error) -> FakeError {
    FakeError::Io(e.to_string())
}
