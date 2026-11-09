//! Servers: unix socket (`0600`, `SO_PEERCRED`) and TCP (by setting, toggled without restart).

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::health::HealthControl;
use crate::hub::Hub;
use crate::limiter::Limiter;
use crate::metrics::Metrics;
use crate::peer::{PeerInfo, TcpPeerListener, UnixPeerListener};
use crate::routes::router;
use crate::state::{ApiState, OriginPolicy, Services};
use agentd_core::{Authenticator, Clock, SystemClock};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `$XDG_RUNTIME_DIR/agentd.sock` in production; overridable by flag and variable (A1-04).
    pub socket_path: PathBuf,
    /// Expected peer uid on the socket; foreign connections are closed.
    pub expected_uid: u32,
    /// TCP address, default `127.0.0.1:4096`.
    pub tcp_addr: SocketAddr,
    /// Initial value of `tunnels.api_external`; off by default.
    pub tcp_enabled: bool,
    /// Additional allowed `Host` values (tunnel addresses).
    pub allowed_hosts: Vec<String>,
    /// Allowed `Origin` values; empty means any browser `Origin` is rejected.
    pub allowed_origins: Vec<String>,
    /// Trusted proxies for `X-Forwarded-For` (loopback by default; the tunnel address is added by a setting).
    pub trusted_proxies: Vec<IpAddr>,
    /// Browser client directory; `None` means statics are off (ADR-0010).
    pub web_root: Option<PathBuf>,
    /// Origin for dev-CORS (development only); also allowed by the `Origin` check.
    pub dev_cors: Vec<String>,
    /// Time source for rate-limit windows.
    pub clock: Arc<dyn Clock>,
    /// How much silence before the core sends a WS ping (docs/reference/api.md 7: 15 s).
    pub ws_ping_interval: Duration,
}

impl ServerConfig {
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> ServerConfig {
        ServerConfig {
            socket_path: socket_path.into(),
            expected_uid: agentd_sys::current_uid(),
            tcp_addr: SocketAddr::from(([127, 0, 0, 1], 4096)),
            tcp_enabled: false,
            allowed_hosts: Vec::new(),
            allowed_origins: Vec::new(),
            trusted_proxies: vec![
                IpAddr::from([127, 0, 0, 1]),
                IpAddr::from([0u16, 0, 0, 0, 0, 0, 0, 1]),
            ],
            web_root: None,
            dev_cors: Vec::new(),
            clock: Arc::new(SystemClock),
            ws_ping_interval: Duration::from_secs(15),
        }
    }
}

/// Runtime control of the TCP listener (setting `tunnels.api_external`).
#[derive(Clone)]
pub struct TcpControl {
    want: watch::Sender<bool>,
    bound: watch::Receiver<Option<SocketAddr>>,
}

impl TcpControl {
    pub fn set_enabled(&self, on: bool) {
        self.want.send_replace(on);
    }

    /// Address the TCP listener is on now (`None` if not listening).
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.bound.borrow()
    }
}

pub struct Server {
    state: Arc<ApiState>,
    socket_path: PathBuf,
    tcp: TcpControl,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

/// The socket is created under a temporary name, gets `0600` and is atomically renamed: the final path never has wider permissions.
/// A live socket at this path is not taken over; the file of a dead socket is replaced.
fn bind_unix(path: &Path) -> io::Result<UnixListener> {
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "agentd is already listening on this socket",
        ));
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.new", std::process::id()));
    let tmp = PathBuf::from(tmp);
    if tmp.exists() {
        std::fs::rename(&tmp, tmp.with_extension("old"))?;
    }
    let listener = UnixListener::bind(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(listener)
}

impl Server {
    /// Starts listeners with ready services (migrations are already done).
    pub async fn start(cfg: ServerConfig, auth: Arc<dyn Authenticator>) -> io::Result<Server> {
        let server = Server::start_booting(cfg).await?;
        server.set_ready(Services::new(auth));
        Ok(server)
    }

    /// Starts listeners before the DB is ready: `GET /v1/health` answers (`starting` or `migrating`), the other
    /// routes return `503`, WS is not accepted. Services are plugged in by `set_ready`.
    pub async fn start_booting(cfg: ServerConfig) -> io::Result<Server> {
        let listener = bind_unix(&cfg.socket_path)?;
        let (want_tx, want_rx) = watch::channel(cfg.tcp_enabled);
        let (bound_tx, bound_rx) = watch::channel(None);
        let tcp = TcpControl {
            want: want_tx,
            bound: bound_rx,
        };
        let state = Arc::new(ApiState {
            policy: OriginPolicy {
                allowed_hosts: cfg.allowed_hosts.clone(),
                allowed_origins: cfg.allowed_origins.clone(),
                trusted_proxies: cfg.trusted_proxies.clone(),
                web_root: cfg.web_root.clone(),
                dev_cors: cfg.dev_cors.clone(),
            },
            services: RwLock::new(None),
            clock: cfg.clock.clone(),
            limiter: Mutex::new(Limiter::default()),
            health: HealthControl::default(),
            metrics: Metrics::default(),
            tcp: tcp.clone(),
            hub: Hub::new(cfg.clock.clone()),
            ping_interval: cfg.ws_ping_interval,
        });
        let app = router(state.clone());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let unix_task = {
            let app = app.clone();
            let mut stop = shutdown_rx.clone();
            let peer_listener = UnixPeerListener {
                inner: listener,
                expected_uid: cfg.expected_uid,
                closed: shutdown_rx.clone(),
            };
            tokio::spawn(async move {
                let serve = axum::serve(
                    peer_listener,
                    app.into_make_service_with_connect_info::<PeerInfo>(),
                )
                .with_graceful_shutdown(async move {
                    drop(stop.wait_for(|v| *v).await);
                });
                if let Err(e) = serve.await {
                    tracing::error!(event = "serve_failed", transport = "unix", kind = ?e.kind(), "server stopped with an error");
                }
            })
        };
        let tcp_task = tokio::spawn(tcp_supervisor(
            cfg.tcp_addr,
            want_rx,
            bound_tx,
            app,
            shutdown_rx,
        ));
        Ok(Server {
            state,
            socket_path: cfg.socket_path,
            tcp,
            shutdown: shutdown_tx,
            tasks: vec![unix_task, tcp_task],
        })
    }

    /// Plugs in services and moves the core to `ok`: from this moment routes accept requests.
    pub fn set_ready(&self, services: Services) {
        crate::topics::register(&self.state.hub, &services);
        *self
            .state
            .services
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(services));
        self.state.health.set_ready();
    }

    /// WS hub: publishing to topics and token revocation.
    #[must_use]
    pub fn hub(&self) -> Hub {
        self.state.hub.clone()
    }

    /// State for `/health`: migration progress and the "degraded" flag.
    #[must_use]
    pub fn health(&self) -> HealthControl {
        self.state.health.clone()
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    #[must_use]
    pub fn tcp(&self) -> TcpControl {
        self.tcp.clone()
    }

    /// Stops listeners; open WS get close `1001`.
    pub async fn shutdown(self) {
        self.shutdown.send_replace(true);
        for t in self.tasks {
            if tokio::time::timeout(Duration::from_secs(5), t)
                .await
                .is_err()
            {
                tracing::warn!(
                    event = "shutdown_timeout",
                    "listener did not stop within 5 s"
                );
            }
        }
    }
}

/// Starts and stops the TCP listener per the value of `want`; on stop closes open TCP WS with code `1001`.
async fn tcp_supervisor(
    addr: SocketAddr,
    mut want: watch::Receiver<bool>,
    bound: watch::Sender<Option<SocketAddr>>,
    app: axum::Router,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        if !*want.borrow() {
            tokio::select! {
                _ = want.changed() => {}
                _ = shutdown.changed() => {}
            }
            continue;
        }
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(event = "tcp_bind_failed", kind = ?e.kind(), "TCP failed to start");
                tokio::select! {
                    _ = want.changed() => {}
                    _ = shutdown.changed() => {}
                }
                continue;
            }
        };
        let local = listener.local_addr().ok();
        let (closed_tx, closed_rx) = watch::channel(false);
        bound.send_replace(local);
        let (mut w, mut s) = (want.clone(), shutdown.clone());
        let serve = axum::serve(
            TcpPeerListener {
                inner: listener,
                closed: closed_rx,
            },
            app.clone()
                .into_make_service_with_connect_info::<PeerInfo>(),
        )
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = w.wait_for(|v| !*v) => {}
                _ = s.wait_for(|v| *v) => {}
            }
        });
        if let Err(e) = serve.await {
            tracing::error!(event = "serve_failed", transport = "tcp", kind = ?e.kind(), "server stopped with an error");
        }
        closed_tx.send_replace(true);
        bound.send_replace(None);
    }
}
