//! MCP client on `rmcp`: stdio (child process) and Streamable HTTP. The core sees only the `McpConnector` ports
//! and `McpSession`; the `rmcp` crate is not visible beyond this file. The stdio process environment is empty: only `PATH` and the variables
//! declared in the server settings; the process gets no core tokens and no `keys.env`.

use std::process::Stdio;
use std::sync::Arc;

use agentd_core::{
    McpCallResult, McpConnector, McpError, McpServerSpec, McpSession, McpTransport, PortFuture,
    RemoteTool,
};
use rmcp::model::{CallToolRequestParams, ContentBlock, ResourceUpdatedNotificationParam};
// The server log is deprecated in the MCP specification (SEP-2577), but servers still send it: accepted as a trigger source.
#[allow(deprecated)]
use rmcp::model::LoggingMessageNotificationParam;
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, RoleClient, ServiceError, ServiceExt};
use serde_json::Value;
use tokio::sync::mpsc;

/// Default `PATH` of the MCP process; a server may declare its own in `env`.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

#[derive(Debug, Default, Clone)]
pub struct RmcpConnector {
    /// Additional certificate authorities for `https` (private CA); system roots remain.
    ca: Vec<reqwest::Certificate>,
}

/// Environment variable with the PEM file of an additional certificate authority for MCP over `https`.
pub const CA_FILE_ENV: &str = "AGENTD_MCP_CA_FILE";

/// The rustls crypto provider is installed once per process (`ring`): `reqwest` is built without a default provider, and without
/// it building an `https` client panics.
fn ensure_tls_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // A provider already installed by someone else is not an error.
        drop(rustls::crypto::ring::default_provider().install_default());
    });
}

impl RmcpConnector {
    /// Connector with an additional certificate authority from a PEM file (private CA of the MCP server). An unreadable file and a file
    /// without certificates are errors: there is no silent fallback to system roots.
    pub fn with_ca(path: &std::path::Path) -> Result<RmcpConnector, String> {
        let pem = std::fs::read(path)
            .map_err(|e| format!("CA file {}: {:?}", path.display(), e.kind()))?;
        let ca = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|_| format!("CA file {}: not PEM certificates", path.display()))?;
        if ca.is_empty() {
            return Err(format!("CA file {}: no certificates", path.display()));
        }
        Ok(RmcpConnector { ca })
    }

    /// From `AGENTD_MCP_CA_FILE`: unset means system roots only; set but unreadable is a startup error.
    pub fn from_env() -> Result<RmcpConnector, String> {
        match std::env::var_os(CA_FILE_ENV).filter(|v| !v.is_empty()) {
            Some(p) => RmcpConnector::with_ca(std::path::Path::new(&p)),
            None => Ok(RmcpConnector::default()),
        }
    }

    fn http_client(&self) -> Result<reqwest::Client, McpError> {
        ensure_tls_provider();
        reqwest::Client::builder()
            .tls_certs_merge(self.ca.clone())
            .build()
            .map_err(|_| McpError::Transport)
    }
}

/// How many notifications wait for parsing; extras are dropped (a server must not bloat core memory).
const NOTICE_QUEUE: usize = 64;

/// Server notification handler: log, resource update, tool list change; queued as text.
struct Notices(mpsc::Sender<String>);

impl Notices {
    fn push(&self, text: String) {
        drop(self.0.try_send(text));
    }
}

impl ClientHandler for Notices {
    #[allow(deprecated)]
    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.push(match params.data {
            Value::String(s) => s,
            other => other.to_string(),
        });
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.push(format!("resource updated: {}", params.uri));
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.push("tool list changed".to_string());
    }
}

struct Session {
    svc: RunningService<RoleClient, Notices>,
    notices: std::sync::Mutex<Option<mpsc::Receiver<String>>>,
}

fn map_err(e: &ServiceError) -> McpError {
    match e {
        ServiceError::Timeout { .. } => McpError::Timeout,
        ServiceError::McpError(_) | ServiceError::UnexpectedResponse => McpError::Protocol,
        _ => McpError::Transport,
    }
}

/// Process command: empty environment, then `PATH` and the declared variables.
fn command(
    path: &str,
    args: &[String],
    env: &std::collections::BTreeMap<String, String>,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args).env_clear().env("PATH", DEFAULT_PATH);
    cmd.envs(env);
    cmd.kill_on_drop(true);
    cmd
}

impl McpSession for Session {
    fn take_notices(&self) -> Option<mpsc::Receiver<String>> {
        self.notices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn list_tools(&self) -> PortFuture<'_, Result<Vec<RemoteTool>, McpError>> {
        Box::pin(async move {
            let tools = self.svc.list_all_tools().await.map_err(|e| map_err(&e))?;
            Ok(tools
                .into_iter()
                .map(|t| RemoteTool {
                    name: t.name.to_string(),
                    description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                    schema: Value::Object((*t.input_schema).clone()),
                })
                .collect())
        })
    }

    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: &'a Value,
    ) -> PortFuture<'a, Result<McpCallResult, McpError>> {
        Box::pin(async move {
            let mut params = CallToolRequestParams::new(tool.to_string());
            if let Some(obj) = args.as_object() {
                params = params.with_arguments(obj.clone());
            }
            let r = self.svc.call_tool(params).await.map_err(|e| map_err(&e))?;
            let text = r
                .content
                .iter()
                .filter_map(|c| match c {
                    ContentBlock::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(McpCallResult {
                text,
                is_error: r.is_error == Some(true),
            })
        })
    }
}

impl McpConnector for RmcpConnector {
    fn connect<'a>(
        &'a self,
        spec: &'a McpServerSpec,
    ) -> PortFuture<'a, Result<Arc<dyn McpSession>, McpError>> {
        Box::pin(async move {
            let (tx, rx) = mpsc::channel(NOTICE_QUEUE);
            let handler = Notices(tx);
            let svc = match &spec.transport {
                McpTransport::Stdio {
                    command: c,
                    args,
                    env,
                } => {
                    let (proc, _) = TokioChildProcess::builder(command(c, args, env))
                        .stderr(Stdio::null())
                        .spawn()
                        .map_err(|_| McpError::Transport)?;
                    handler.serve(proc).await.map_err(|_| McpError::Transport)?
                }
                McpTransport::Http { url } => {
                    let t = StreamableHttpClientTransport::with_client(
                        self.http_client()?,
                        StreamableHttpClientTransportConfig::with_uri(url.as_str()),
                    );
                    handler.serve(t).await.map_err(|_| McpError::Transport)?
                }
            };
            Ok(Arc::new(Session {
                svc,
                notices: std::sync::Mutex::new(Some(rx)),
            }) as Arc<dyn McpSession>)
        })
    }
}
