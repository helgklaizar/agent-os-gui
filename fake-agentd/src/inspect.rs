//! Observing mock state from tests: request log, echo, WS count.

use crate::{EchoRecord, FakeAgentd, RequestRecord};

impl FakeAgentd {
    /// SHA-256 of all stored tokens in hex; the mock stores nothing else about tokens.
    pub fn stored_token_hashes(&self) -> Vec<String> {
        self.shared.lock().tokens.hashes().unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<RequestRecord> {
        self.shared.lock().requests.clone()
    }

    pub fn echo_log(&self) -> Vec<EchoRecord> {
        self.shared.lock().echo.clone()
    }

    pub fn ws_connections(&self) -> usize {
        self.shared.hub.conns()
    }
}
