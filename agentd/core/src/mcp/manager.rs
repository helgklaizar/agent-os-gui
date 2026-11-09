//! MCP server manager: connecting per the `connections` page, the tool catalog, calls with a timeout, restarting the
//! connection after a failure, server status in the `connections` topic.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::spec::parse_all;
use super::{
    CallResult, McpConnector, McpError, McpNotice, McpNoticeHook, McpSession, RemoteTool,
    ServerSpec, TOOL_PREFIX,
};
use crate::{Action, ExecError, Notifier, SettingsStore};

/// After how many seconds to retry a server that did not come up on connect.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The handshake (process start, `initialize`, `tools/list`) waits at least this long: a short `timeout_secs` sets the deadline
/// of a tool call, not of server start, otherwise a slow start turns the tool into `unknown_tool`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Connected,
    Down,
}

impl ServerStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ServerStatus::Connected => "connected",
            ServerStatus::Down => "down",
        }
    }
}

/// Catalog tool: where to call and which action to check against permissions.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    pub server: String,
    pub tool: String,
    pub action: Action,
    pub timeout: Duration,
    pub description: String,
    pub schema: Value,
}

impl McpTool {
    /// Call target for the permission card and the log: `<server>/<tool>`.
    #[must_use]
    pub fn target(&self) -> String {
        format!("{}/{}", self.server, self.tool)
    }
}

struct Server {
    spec: ServerSpec,
    session: Option<Arc<dyn McpSession>>,
    tools: Vec<RemoteTool>,
    last_try: Option<Instant>,
    /// A connection is in progress: the receiver closes when it ends (successfully or not).
    connecting: Option<tokio::sync::watch::Receiver<()>>,
    announced: Option<(ServerStatus, usize)>,
}

impl Server {
    fn status(&self) -> ServerStatus {
        if self.session.is_some() {
            ServerStatus::Connected
        } else {
            ServerStatus::Down
        }
    }
}

/// Connect guard: if `run_connect` is cut short by a panic, it records the failure and clears `connecting`.
struct ConnectGuard<'a> {
    mcp: &'a Mcp,
    spec: &'a ServerSpec,
    armed: bool,
}

impl Drop for ConnectGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.mcp.finish(self.spec, None);
        }
    }
}

struct Inner {
    settings: Arc<dyn SettingsStore>,
    connector: Arc<dyn McpConnector>,
    notifier: Arc<dyn Notifier>,
    servers: Mutex<Vec<Server>>,
    sync_lock: tokio::sync::Mutex<()>,
    hook: std::sync::OnceLock<Arc<dyn McpNoticeHook>>,
}

#[derive(Clone)]
pub struct Mcp {
    inner: Arc<Inner>,
}

impl Mcp {
    #[must_use]
    pub fn new(
        settings: Arc<dyn SettingsStore>,
        connector: Arc<dyn McpConnector>,
        notifier: Arc<dyn Notifier>,
    ) -> Mcp {
        Mcp {
            inner: Arc::new(Inner {
                settings,
                connector,
                notifier,
                servers: Mutex::new(Vec::new()),
                sync_lock: tokio::sync::Mutex::new(()),
                hook: std::sync::OnceLock::new(),
            }),
        }
    }

    /// Server notification receiver (one per core; a repeated call is ignored).
    pub fn set_notice_hook(&self, hook: Arc<dyn McpNoticeHook>) {
        drop(self.inner.hook.set(hook));
    }

    /// Forwards session notifications to the receiver while the connection is alive.
    fn forward_notices(&self, server: &str, session: &Arc<dyn McpSession>) {
        let (Some(hook), Some(mut rx)) = (self.inner.hook.get().cloned(), session.take_notices())
        else {
            return;
        };
        let server = server.to_string();
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                hook.on_notice(&McpNotice {
                    server: server.clone(),
                    text,
                })
                .await;
            }
        });
    }

    /// Background reconcile with the settings page: status is visible without calls.
    pub fn spawn(&self, every: Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                drop(me.start_connects().await);
                tokio::time::sleep(every).await;
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Server>> {
        self.inner
            .servers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn open(&self, spec: &ServerSpec) -> Option<(Arc<dyn McpSession>, Vec<RemoteTool>)> {
        let limit = spec.timeout.max(CONNECT_TIMEOUT);
        let session = tokio::time::timeout(limit, self.inner.connector.connect(spec))
            .await
            .ok()?
            .ok()?;
        let tools = tokio::time::timeout(limit, session.list_tools())
            .await
            .ok()?
            .ok()?;
        Some((session, tools))
    }

    /// Brings connections in line with the `connections` page: new servers connect, changed ones
    /// restart, removed and disabled ones close, failed ones are retried no more than once per `RETRY_AFTER`.
    /// Returns when all started connections have finished.
    pub async fn sync(&self) {
        for h in self.start_connects().await {
            drop(h.await);
        }
    }

    /// Reconcile with the settings page and start connections in the background: waits for no handshake.
    async fn start_connects(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();
        for (spec, tx) in self.reconcile().await {
            let me = self.clone();
            handles.push(tokio::spawn(async move { me.run_connect(spec, tx).await }));
        }
        handles
    }

    /// Fast part of the reconcile: the server list updates at once, connections are returned as tasks and run
    /// outside the locks, so a hung server blocks neither the reconcile nor its neighbours' calls.
    async fn reconcile(&self) -> Vec<(ServerSpec, tokio::sync::watch::Sender<()>)> {
        let _guard = self.inner.sync_lock.lock().await;
        let Ok(page) = self.inner.settings.get("connections").await else {
            return Vec::new();
        };
        let mut errs = Vec::new();
        let specs: Vec<ServerSpec> = parse_all(&page.doc, &mut errs)
            .into_iter()
            .filter(|s| s.enabled)
            .collect();
        let mut jobs = Vec::new();
        let mut old: Vec<Server> = std::mem::take(&mut *self.lock());
        let mut next = Vec::new();
        for spec in specs {
            let kept = old
                .iter()
                .position(|s| s.spec == spec)
                .map(|i| old.swap_remove(i));
            let mut server = kept.unwrap_or(Server {
                spec: spec.clone(),
                session: None,
                tools: Vec::new(),
                last_try: None,
                connecting: None,
                announced: None,
            });
            let due = server.last_try.is_none_or(|t| t.elapsed() >= RETRY_AFTER);
            if server.session.is_none() && server.connecting.is_none() && due {
                server.last_try = Some(Instant::now());
                let (tx, rx) = tokio::sync::watch::channel(());
                server.connecting = Some(rx);
                jobs.push((spec, tx));
            }
            next.push(server);
        }
        drop(old);
        for s in &mut next {
            self.announce(s);
        }
        *self.lock() = next;
        jobs
    }

    /// Connects one server and records the result if the server is still listed with the same description.
    async fn run_connect(&self, spec: ServerSpec, done: tokio::sync::watch::Sender<()>) {
        // A panic in `open` unwinds the stack: the guard clears `connecting`, and the reconcile retries the connection after `RETRY_AFTER`.
        let mut guard = ConnectGuard {
            mcp: self,
            spec: &spec,
            armed: true,
        };
        let opened = self.open(&spec).await;
        if opened.is_none() {
            tracing::warn!(
                scope = "mcp",
                event = "connect_failed",
                "MCP server not connected"
            );
        }
        guard.armed = false;
        self.finish(&spec, opened);
        drop(done);
    }

    /// Records the connection result and clears the `connecting` flag if the server is still listed with the same description.
    fn finish(&self, spec: &ServerSpec, opened: Option<(Arc<dyn McpSession>, Vec<RemoteTool>)>) {
        let mut servers = self.lock();
        if let Some(s) = servers.iter_mut().find(|s| s.spec == *spec) {
            s.connecting = None;
            if let Some((session, tools)) = opened
                && s.session.is_none()
            {
                self.forward_notices(&spec.name, &session);
                s.session = Some(session);
                s.tools = tools;
            }
            self.announce(s);
        }
    }

    /// Connections still in progress (all or one server's): the server deadline and the completion receiver.
    fn pending(&self, only: Option<&str>) -> Vec<(Duration, tokio::sync::watch::Receiver<()>)> {
        self.lock()
            .iter()
            .filter(|s| only.is_none_or(|n| s.spec.name == n))
            .filter_map(|s| Some((s.spec.timeout, s.connecting.clone()?)))
            .collect()
    }

    fn announce(&self, s: &mut Server) {
        let now = (s.status(), s.tools.len());
        if s.announced == Some(now) {
            return;
        }
        s.announced = Some(now);
        self.inner.notifier.publish(
            "connections",
            json!({"kind": "mcp", "server": s.spec.name, "status": now.0.as_str(), "tools": now.1}),
        );
    }

    /// Tool by the model-facing name (`mcp__<server>__<tool>`); no such server or tool gives `None`.
    pub async fn resolve(&self, name: &str) -> Option<McpTool> {
        let rest = name.strip_prefix(TOOL_PREFIX)?;
        drop(self.start_connects().await);
        // Wait only for the connection of the server the call is addressed to: another server's handshake does not delay it.
        let target = self
            .lock()
            .iter()
            .find(|s| {
                rest.strip_prefix(s.spec.name.as_str())
                    .is_some_and(|r| r.starts_with("__"))
            })
            .map(|s| s.spec.name.clone());
        if let Some(target) = target {
            for (_, mut rx) in self.pending(Some(&target)) {
                while rx.changed().await.is_ok() {}
            }
        }
        let servers = self.lock();
        servers.iter().find_map(|s| {
            let tool = rest
                .strip_prefix(s.spec.name.as_str())?
                .strip_prefix("__")?;
            let t = s.tools.iter().find(|t| t.name == tool)?;
            s.session.as_ref()?;
            Some(McpTool {
                server: s.spec.name.clone(),
                tool: t.name.clone(),
                action: s.spec.risk_of(&t.name),
                timeout: s.spec.timeout,
                description: t.description.clone(),
                schema: t.schema.clone(),
            })
        })
    }

    /// Catalog for the model: tools of all connected servers under the names `mcp__<server>__<tool>`.
    /// The reconcile with the settings page runs only while there are no servers yet; after that the catalog takes the state of the background reconcile
    /// (`spawn`): a model request does not restart a failed server ahead of time. The catalog waits for a connection in progress no longer than
    /// its server's `timeout_secs`: a hung server does not hold up the model turn.
    pub async fn catalog(&self) -> Vec<crate::ToolSpec> {
        if self.lock().is_empty() {
            drop(self.start_connects().await);
        }
        let waits: Vec<_> = self
            .pending(None)
            .into_iter()
            .map(|(limit, mut rx)| {
                tokio::spawn(async move {
                    drop(
                        tokio::time::timeout(limit, async { while rx.changed().await.is_ok() {} })
                            .await,
                    );
                })
            })
            .collect();
        for w in waits {
            drop(w.await);
        }
        self.lock()
            .iter()
            .filter(|s| s.session.is_some())
            .flat_map(|s| {
                s.tools.iter().map(|t| crate::ToolSpec {
                    name: super::tool_name(&s.spec.name, &t.name),
                    description: t.description.clone(),
                    schema: t.schema.clone(),
                })
            })
            .collect()
    }

    fn session_of(&self, server: &str) -> Option<Arc<dyn McpSession>> {
        self.lock()
            .iter()
            .find(|s| s.spec.name == server)
            .and_then(|s| s.session.clone())
    }

    /// Closes the connection if it is still the same one: the next reconcile starts the server again.
    fn drop_session(&self, server: &str, session: &Arc<dyn McpSession>) {
        let mut servers = self.lock();
        if let Some(s) = servers.iter_mut().find(|s| s.spec.name == server)
            && s.session.as_ref().is_some_and(|x| Arc::ptr_eq(x, session))
        {
            s.session = None;
            s.last_try = None;
            self.announce(s);
        }
    }

    /// Tool call. Permissions are not checked here: the dispatcher did that. A link failure or timeout closes the connection;
    /// the call is not repeated (the outcome is unknown, a double side effect is possible).
    pub async fn call(&self, tool: &McpTool, args: &Value) -> Result<CallResult, ExecError> {
        let Some(session) = self.session_of(&tool.server) else {
            return Err(ExecError {
                code: "mcp_unavailable",
            });
        };
        let res = tokio::time::timeout(tool.timeout, session.call(&tool.tool, args)).await;
        let err = match res {
            Ok(Ok(r)) => return Ok(r),
            Ok(Err(McpError::Protocol)) => {
                return Err(ExecError { code: "mcp_error" });
            }
            Ok(Err(McpError::Timeout)) | Err(_) => "timeout",
            Ok(Err(McpError::Transport)) => "mcp_unavailable",
        };
        self.drop_session(&tool.server, &session);
        Err(ExecError { code: err })
    }
}
