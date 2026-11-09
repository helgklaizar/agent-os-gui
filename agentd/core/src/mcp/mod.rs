//! Core MCP client: servers from the `connections` page (key `mcp`), the tool catalog,
//! calls through the action dispatcher (permissions and log live there). The transport (stdio, Streamable HTTP) sits behind the
//! `McpConnector` port, implemented by the `agentd-providers` crate. A tool result is untrusted text.

mod manager;
pub mod spec;

use std::sync::Arc;

use serde_json::Value;

use crate::PortFuture;

pub use manager::{Mcp, McpTool, ServerStatus};
pub use spec::{DEFAULT_RISK, ServerSpec, TOOL_PREFIX, Transport, tool_name};

/// A tool as declared by the server.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// Call result: the joined reply text and the tool error flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// The server did not answer in the allotted time.
    Timeout,
    /// The process or connection is unavailable.
    Transport,
    /// The server answered with a protocol error.
    Protocol,
}

/// An MCP server notification (server log, resource update, tool list change) as text.
/// The text is untrusted: it comes from an external process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpNotice {
    /// Server name from the `connections` page.
    pub server: String,
    pub text: String,
}

/// Server notification receiver: triggers (source `mcp`).
pub trait McpNoticeHook: Send + Sync + 'static {
    fn on_notice<'a>(&'a self, notice: &'a McpNotice) -> PortFuture<'a, ()>;
}

/// A live connection to one server.
pub trait McpSession: Send + Sync + 'static {
    /// Stream of server notification texts; handed out once after connecting. No notifications gives `None`.
    fn take_notices(&self) -> Option<tokio::sync::mpsc::Receiver<String>> {
        None
    }

    fn list_tools(&self) -> PortFuture<'_, Result<Vec<RemoteTool>, McpError>>;
    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: &'a Value,
    ) -> PortFuture<'a, Result<CallResult, McpError>>;
}

/// Creates a connection from the server description (process start, handshake).
pub trait McpConnector: Send + Sync + 'static {
    fn connect<'a>(
        &'a self,
        spec: &'a ServerSpec,
    ) -> PortFuture<'a, Result<Arc<dyn McpSession>, McpError>>;
}
