//! Mock of the `agentd` core for tests and manual development.
//!
//! The library starts inside a test ([`FakeAgentd::start`]); the binary is for manual work and UI scenarios.
//! The contract is `api.md` and `openapi-draft.yaml`; the mock does not depend on `core-client`: it describes frames and errors itself,
//! otherwise a shared error in the types would be visible to neither client nor mock. The `/v1/_test/*` service routes and the
//! extension fields in `POST /v1/tokens` exist only here and are not part of the contract.

pub mod admin;
pub mod agent_sim;
pub mod auth;
pub mod bar;
pub mod bar_http;
pub mod chat_ctl;
pub mod chat_http;
pub mod chat_msgs;
pub mod chat_state;
pub mod clock;
pub mod config;
pub mod db;
pub mod disk;
pub mod events_api;
pub mod events_db;
pub mod events_http;
pub mod events_ops;
pub mod events_registry;
pub mod events_seed;
pub mod events_sim;
pub mod handlers;
pub mod hub;
pub mod indicators;
pub mod inspect;
pub mod limiter;
mod listen;
pub mod log_http;
pub mod mask;
pub mod ops_http;
pub mod perm_flow;
pub mod perm_http;
pub mod perm_store;
pub mod perm_sweep;
pub mod reply;
pub mod routes;
pub mod scenario;
pub mod search;
pub mod search_http;
pub mod server;
pub mod sess_http;
pub mod settings_store;
pub mod sim;
pub mod state;
pub mod store;
pub mod tasks_http;
pub mod tasks_ops;
pub mod term_sim;
pub mod token;
pub mod tools_api;
pub mod tools_http;
pub mod tools_state;
pub mod tree;
pub mod tree_http;
pub mod tree_ops;
pub mod tree_pub;
pub mod tree_seed;
pub mod tree_session_ops;
pub mod types;
pub mod ws;

use std::net::SocketAddr;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::task::JoinHandle;

pub use admin::*;
pub use clock::{Clock, ClockMode};
pub use config::{Config, CoreMode};
pub use db::{
    Db, DbError, FreeSpace, MIGRATIONS, Migration, StorageReport, migrate, migrate_with_space,
};
pub use disk::{DiskEncryption, DiskProbe, DiskWarning, Mechanism};
pub use scenario::{Action, Scenario, ScenarioError, Step, StepKind};
pub use store::{IssuedToken, Kind, Scope, StoreError, TokenSpec};

use state::Shared;

use listen::{accept_tcp, accept_unix, write_token_file};
use types::io;
pub use types::{EchoRecord, FakeError, FrameType, RequestRecord, Transport};

pub struct FakeAgentd {
    dir: tempfile::TempDir,
    socket: PathBuf,
    tcp: Option<SocketAddr>,
    admin: String,
    db_path: PathBuf,
    db: Arc<Db>,
    shared: Arc<Shared>,
    tasks: Vec<JoinHandle<()>>,
}

impl FakeAgentd {
    pub async fn start(config: Config) -> Result<FakeAgentd, FakeError> {
        let dir = tempfile::Builder::new()
            .prefix("fa-")
            .tempdir()
            .map_err(io)?;
        let socket = config
            .socket
            .clone()
            .unwrap_or_else(|| dir.path().join("agentd.sock"));
        let db_path = config
            .db
            .clone()
            .unwrap_or_else(|| dir.path().join("sessions.db"));
        let data_dir = db_path.parent().unwrap_or(dir.path()).to_path_buf();
        let db = Arc::new(Db::open(&db_path).map_err(|e| FakeError::Config(format!("DB: {e}")))?);
        let shared = Arc::new(
            Shared::new(
                Clock::new(config.clock),
                config.api_major,
                config.allowed_origins.clone(),
                db.clone(),
                config.reply_delay,
                data_dir,
            )
            .map_err(|e| FakeError::Config(e.to_string()))?,
        );
        files_http::seed(&shared.data_dir).map_err(io)?;
        chat_msgs::interrupt_streaming(&shared)
            .map_err(|e| FakeError::Config(format!("DB: {e}")))?;
        bar::start(&shared, &db).map_err(|e| FakeError::Config(format!("bar fixtures: {e}")))?;
        tree_ops::start(&shared, &db)
            .map_err(|e| FakeError::Config(format!("tree fixtures: {e}")))?;
        if config.event_fixtures {
            events_seed::start(&shared, &db)
                .map_err(|e| FakeError::Config(format!("event fixtures: {e}")))?;
        }
        let now = shared.clock.now_unix();
        let admin = shared
            .lock()
            .tokens
            .issue(&TokenSpec::new(Scope::Admin, Kind::Test), now, None)
            .map_err(|e| FakeError::Config(e.to_string()))?
            .token;
        if let Some(path) = &config.token_file {
            let gui = shared
                .lock()
                .tokens
                .issue(&TokenSpec::new(Scope::Admin, Kind::Gui), now, None)
                .map_err(|e| FakeError::Config(e.to_string()))?;
            write_token_file(path, &gui.token)?;
        }
        let mut tasks = Vec::new();
        let listener = UnixListener::bind(&socket).map_err(io)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
        let owner_uid = std::fs::metadata(&socket).map_err(io)?.uid();
        tasks.push(tokio::spawn(accept_unix(
            shared.clone(),
            listener,
            owner_uid,
        )));
        let mut tcp = None;
        if let Some(addr) = config.tcp {
            let l = TcpListener::bind(addr).await.map_err(io)?;
            let local = l.local_addr().map_err(io)?;
            shared.set_tcp_port(local.port());
            tcp = Some(local);
            tasks.push(tokio::spawn(accept_tcp(shared.clone(), l)));
        }
        if let Some(period) = config.bar_sys_period {
            tasks.push(tokio::spawn(sim::ticker(
                shared.clone(),
                period,
                config.bar_seed,
            )));
        }
        if let Some(sc) = config.scenario {
            tasks.push(tokio::spawn(sim::play(shared.clone(), sc)));
        }
        Ok(FakeAgentd {
            dir,
            socket,
            tcp,
            admin,
            db_path,
            db,
            shared,
            tasks,
        })
    }

    /// Temporary directory of the mock (removed with it).
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// Mock DB file (`sessions.db`, schema `0001_init.sql`).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// The single DB writer of the mock: tests change tables through it, then call [`publish_tables`](Self::publish_tables).
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Last state of a topic (what a new subscriber gets as the first frame).
    pub fn snapshot(&self, topic: &str) -> serde_json::Value {
        self.shared.hub.snapshot(topic)
    }

    /// A session's unread count grew by `n` (in the real core a message arrival does this); an `unread` frame goes to subscribers.
    pub fn add_unread(&self, session_id: i64, n: i64) -> Result<(), DbError> {
        tree_session_ops::add_unread(&self.shared, session_id, n).map_err(|e| match e {
            tree_ops::TreeError::Db(m) => DbError::Io(std::io::Error::other(m)),
            other => DbError::Io(std::io::Error::other(format!("{other:?}"))),
        })
    }

    /// Re-reads `quotas`, `accounts`, `indicators` and publishes the `bar.quota` and `bar.indicators` snapshots.
    pub fn publish_tables(&self) -> Result<(), DbError> {
        bar::publish_tables(&self.shared, &self.db)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn tcp_addr(&self) -> Option<SocketAddr> {
        self.tcp
    }

    /// An `admin` token of kind `test`, issued at startup.
    pub fn admin_token(&self) -> &str {
        &self.admin
    }

    pub fn issue(&self, spec: TokenSpec) -> Result<IssuedToken, StoreError> {
        let now = self.shared.clock.now_unix();
        self.shared.lock().tokens.issue(&spec, now, None)
    }

    /// Token revocation; open WS connections using it are closed with code 4401.
    pub fn revoke(&self, id: i64) {
        let now = self.shared.clock.now_unix();
        if matches!(self.shared.lock().tokens.revoke(id, now), Ok(true)) {
            self.shared.hub.control(hub::Out::Revoke(id));
        }
    }

    /// The topic returns "no data" (all fields `null`) until the next explicit publication.
    pub fn no_data(&self, topic: &str) {
        self.shared.hub.no_data(topic);
    }

    /// Drops all WS without a close frame; new WS are not accepted until [`restore`](Self::restore).
    pub fn drop_ws(&self) {
        self.shared.hub.drop_ws();
    }

    pub fn restore(&self) {
        self.shared.hub.restore();
    }

    pub fn set_mode(&self, mode: CoreMode) {
        self.shared.set_mode(mode);
    }

    /// `false` - the mock stops answering `pong`.
    pub fn set_pongs(&self, on: bool) {
        self.shared.hub.set_pongs(on);
    }

    /// The next topic frame on each connection gets `seq` one greater than expected.
    pub fn inject_seq_gap(&self, topic: &str) {
        self.shared
            .hub
            .control(hub::Out::SkipSeq(topic.to_string()));
    }

    /// Publishes a frame to subscribers and remembers it as the topic's last state (for snapshots).
    pub fn emit(&self, topic: &str, kind: FrameType, data: serde_json::Value) {
        self.shared.hub.emit(topic, kind, data);
    }

    /// Advances the virtual clock; token expiry, the attempt limit and scenario steps follow it.
    pub fn advance(&self, by: Duration) {
        self.shared.clock.advance(by);
        perm_sweep::sweep_expired(&self.shared);
    }

    pub fn now_unix(&self) -> i64 {
        self.shared.clock.now_unix()
    }
}

impl Drop for FakeAgentd {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}
