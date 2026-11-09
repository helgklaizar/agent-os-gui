//! Helpers for A2 end-to-end tests: the real `agentd` binary on a temporary DB, tokens of three scopes, HTTP and WS through `core-client`.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use agentd_auth::{NewToken, TokenPolicy, TokenStore};
use agentd_core::{Scope as CoreScope, SystemClock, TokenKind};
use agentd_db::{Connection, Db, DbConfig};
use core_client::client::HttpClient;
use core_client::http::Request;
use core_client::{ConnState, CoreClient, CoreConfig, CoreEvent, Endpoint, Token, TokenSource};
use serde_json::{Value, json};

pub type Setup = Box<dyn FnOnce(&Connection) -> agentd_db::SqlResult<()> + Send>;

mod bin;
mod chat;
mod daemon;
mod ev;
mod mcp;
mod perm;
mod raw;
mod runner;
mod tasks;
mod tokens;
mod ws;

#[allow(unused_imports)]
pub use bin::agentd_bin;
#[allow(unused_imports)]
pub use chat::*;
use daemon::Daemon;
#[allow(unused_imports)]
pub use ev::*;
#[allow(unused_imports)]
pub use mcp::*;
#[allow(unused_imports)]
pub use perm::*;
#[allow(unused_imports)]
pub use raw::*;
#[allow(unused_imports)]
pub use runner::*;
#[allow(unused_imports)]
pub use tasks::*;
use tokens::mk_listed;
pub use ws::Ws;

/// Single-token client: the reply is a status and a body (`Null` if there is no body).
pub struct Api(HttpClient);

pub type Reply = (u16, Value);

impl Api {
    async fn run(&self, req: Request) -> Reply {
        let r = self.0.send(req).await.expect("request");
        let body = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        (r.status, body)
    }
    pub async fn get(&self, path: &str) -> Reply {
        self.run(Request::get(&format!("/v1{path}"))).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Reply {
        self.run(Request::post(&format!("/v1{path}"), body)).await
    }
    pub async fn patch(&self, path: &str, body: Value) -> Reply {
        self.run(Request::patch(&format!("/v1{path}"), body)).await
    }
    /// `PATCH` with `If-Match` (settings pages).
    pub async fn patch_if_match(&self, path: &str, body: Value, rev: i64) -> Reply {
        self.run(Request::patch(&format!("/v1{path}"), body).header("If-Match", &rev.to_string()))
            .await
    }
    pub async fn put(&self, path: &str, body: Value) -> Reply {
        let mut req = Request::post(&format!("/v1{path}"), body);
        req.method = "PUT".into();
        self.run(req).await
    }
    pub async fn delete(&self, path: &str) -> Reply {
        self.run(Request::delete(&format!("/v1{path}"))).await
    }
    /// Create with a `201` check; returns the body.
    pub async fn make(&self, path: &str, body: Value) -> Value {
        let (st, v) = self.post(path, body).await;
        assert_eq!(st, 201, "POST {path}: {v}");
        v
    }
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub sock: PathBuf,
    pub home: PathBuf,
    /// Core log (stderr), searched for leaks.
    pub log: PathBuf,
    rust_log: String,
    test_clock: bool,
    extra_env: Vec<(String, String)>,
    pub admin: Api,
    pub chat: Api,
    pub read: Api,
    /// A second `admin` (second GUI window), an agent token and a module token (`chat` with a method list).
    pub admin2: Api,
    pub agent: Api,
    pub module: Api,
    pub agent_token: String,
    pub module_token: String,
    read_token: String,
    pub chat_token: String,
    pub admin_token: String,
    daemon: Daemon,
}

pub(super) fn mk(kind: TokenKind, scope: CoreScope) -> NewToken {
    NewToken {
        label: "e2e".into(),
        kind,
        scope,
        ttl_days: None,
        allowlist: None,
        created_by: None,
    }
}

impl Env {
    pub async fn start() -> Env {
        Env::start_with(|_| Ok(())).await
    }

    /// Core with a controllable clock: `Env::advance` moves time (permission request TTL, command timeouts).
    pub async fn start_clock() -> Env {
        Env::launch_full(|_| Ok(()), "info", true, Vec::new()).await
    }

    /// Core with extra environment variables (D-Bus bus, limit windows).
    pub async fn start_with_env(extra: Vec<(String, String)>) -> Env {
        Env::launch_full(|_| Ok(()), "info", false, extra).await
    }

    /// Core with the observer enabled on a model (A7-03): model `scripted:watch`, heartbeat every `heartbeat_secs`.
    pub async fn start_watch(heartbeat_secs: u64) -> Env {
        let extra = vec![
            ("AGENTD_WATCH".to_string(), "1".to_string()),
            (
                "AGENTD_WATCH_MODEL".to_string(),
                "scripted:watch".to_string(),
            ),
            (
                "AGENTD_WATCH_HEARTBEAT_SECS".to_string(),
                heartbeat_secs.to_string(),
            ),
        ];
        Env::launch_full(|_| Ok(()), "info", false, extra).await
    }

    /// Moves the controllable core clock by `secs` seconds.
    pub async fn advance(&self, secs: i64) {
        let (st, v) = self
            .admin
            .post("/_test/advance", json!({"seconds": secs}))
            .await;
        assert_eq!(st, 200, "{v}");
    }

    /// Core with a `trace` log: "no secret in the logs" checks.
    pub async fn start_trace() -> Env {
        Env::launch(|_| Ok(()), "trace").await
    }

    /// `setup` runs on the DB before the core starts: fixtures that cannot be created through the API (no routes yet,
    /// secrets, bookmarks, counters).
    pub async fn start_with(
        setup: impl FnOnce(&Connection) -> agentd_db::SqlResult<()> + Send + 'static,
    ) -> Env {
        Env::launch(setup, "info").await
    }

    async fn launch(
        setup: impl FnOnce(&Connection) -> agentd_db::SqlResult<()> + Send + 'static,
        rust_log: &str,
    ) -> Env {
        Env::launch_full(setup, rust_log, false, Vec::new()).await
    }

    async fn launch_full(
        setup: impl FnOnce(&Connection) -> agentd_db::SqlResult<()> + Send + 'static,
        rust_log: &str,
        test_clock: bool,
        extra_env: Vec<(String, String)>,
    ) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("agentos/sessions.db");
        let sock = dir.path().join("agentd.sock");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let (admin, chat, read, extra) = {
            let handle = Arc::new(Db::open(DbConfig::new(&db)).unwrap());
            handle.write(move |tx| setup(tx)).await.unwrap();
            let store = TokenStore::new(handle, Arc::new(SystemClock), TokenPolicy::default());
            let a = store
                .issue(mk(TokenKind::Gui, CoreScope::Admin))
                .await
                .unwrap();
            let c = store
                .issue(mk(TokenKind::External, CoreScope::Chat))
                .await
                .unwrap();
            let r = store
                .issue(mk(TokenKind::External, CoreScope::Read))
                .await
                .unwrap();
            let a2 = store
                .issue(mk(TokenKind::Gui, CoreScope::Admin))
                .await
                .unwrap();
            let ag = store.issue(mk_listed(TokenKind::Agent)).await.unwrap();
            let md = store.issue(mk_listed(TokenKind::Module)).await.unwrap();
            (
                a.value.expose().clone(),
                c.value.expose().clone(),
                r.value.expose().clone(),
                [
                    a2.value.expose().clone(),
                    ag.value.expose().clone(),
                    md.value.expose().clone(),
                ],
            )
        };
        let log = dir.path().join("agentd.log");
        let daemon = Daemon::start(&db, &sock, &home, &log, rust_log, test_clock, &extra_env);
        let ep = Endpoint::Unix(sock.clone());
        let api = |t: &str| Api(HttpClient::new(ep.clone(), Some(Token::new(t.to_string()))));
        Env {
            admin: api(&admin),
            chat: api(&chat),
            read: api(&read),
            admin2: api(&extra[0]),
            agent: api(&extra[1]),
            module: api(&extra[2]),
            agent_token: extra[1].clone(),
            module_token: extra[2].clone(),
            read_token: read,
            chat_token: chat,
            admin_token: admin,
            dir,
            db,
            sock,
            home,
            log,
            rust_log: rust_log.to_string(),
            test_clock,
            extra_env,
            daemon,
        }
    }

    /// Client for an arbitrary token over this core's socket.
    pub fn api_with(&self, token: &str) -> Api {
        Api(HttpClient::new(
            Endpoint::Unix(self.sock.clone()),
            Some(Token::new(token.to_string())),
        ))
    }

    /// WS client of the `read` token: events arrive on the `rx` channel.
    pub fn ws(&self) -> Ws {
        self.ws_as(&self.read_token)
    }

    /// WS client for the given token.
    pub fn ws_as(&self, token: &str) -> Ws {
        let c = CoreClient::start(CoreConfig::new(
            Endpoint::Unix(self.sock.clone()),
            TokenSource::Value(Token::new(token.to_string())),
        ))
        .unwrap();
        let ws = Ws::new(c);
        ws.wait(|e| matches!(e, CoreEvent::State(ConnState::Connected)));
        ws
    }

    /// Core crash (`SIGKILL`) and a new start on the same DB: what a restart after a failure looks like.
    pub fn crash_and_restart(&mut self) {
        self.daemon.crash();
        let _ = std::fs::remove_file(&self.sock);
        self.daemon = Daemon::start(
            &self.db,
            &self.sock,
            &self.home,
            &self.log,
            &self.rust_log,
            self.test_clock,
            &self.extra_env,
        );
    }

    /// Stops the core and reads the DB directly: checks of state the API does not show.
    pub async fn inspect<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(&Connection) -> agentd_db::SqlResult<T> + Send + 'static,
    ) -> T {
        self.daemon.stop();
        let db = Db::open(DbConfig::new(&self.db)).unwrap();
        let out = db.read(f).await.unwrap();
        drop(db);
        self.daemon = Daemon::start(
            &self.db,
            &self.sock,
            &self.home,
            &self.log,
            &self.rust_log,
            self.test_clock,
            &self.extra_env,
        );
        out
    }
}

pub fn fields(v: &Value) -> Vec<String> {
    v["error"]["details"]["fields"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| f["field"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn persona(name: &str) -> Value {
    json!({ "name": name, "color": "#fff" })
}
