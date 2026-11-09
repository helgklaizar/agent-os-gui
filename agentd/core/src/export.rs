//! Port for encrypted session export: implemented by `agentd-db` (reading the session and `age` encryption).

use crate::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportInfo {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExportError {
    NotFound,
    /// The recipient did not parse as an `age` key (`age1…`).
    BadRecipient,
    Unavailable,
}

pub trait SessionExporter: Send + Sync + 'static {
    /// Writes `session-<id>-<ts>.json.age` to the export directory; creates no plain file.
    fn export(
        &self,
        session_id: i64,
        recipients: Vec<String>,
        now: i64,
    ) -> PortFuture<'_, Result<ExportInfo, ExportError>>;
}
