//! A real `agentd` binary (`scripted` provider) on a temporary DB and a GUI client against it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentd_auth::{NewToken, TokenPolicy, TokenStore};
use agentd_core::{Scope, SystemClock, TokenKind};
use agentd_db::{Db, DbConfig};
use core_client::http::Request;
use core_client::{
    CoreClient, CoreConfig, CoreEvent, Endpoint, HttpClient, ReconnectPolicy, Token, TokenSource,
};
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(15);

fn agentd_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("exe");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>");
    let bin = dir.join("agentd");
    // `cargo test` does not rebuild the core binary: a stale `agentd` answers differently from what the
    // scenarios expect, and tests turn red for no clear reason. Build a fresh one once per process.
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args([
            "build",
            "--locked",
            "-p",
            "agentd",
            "--features",
            "test-support",
        ])
        .arg("--target-dir")
        .arg(dir.parent().expect("target"));
        if dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo build -p agentd");
        assert!(
            status.success(),
            "cargo build -p agentd did not pass: {status}"
        );
    });
    assert!(
        bin.exists(),
        "no {}: build `cargo build -p agentd --features test-support`",
        bin.display()
    );
    bin
}

fn spawn(db: &Path, sock: &Path, home: &Path, log: &Path) -> Child {
    Command::new(agentd_bin())
        .args(["serve", "--db"])
        .arg(db)
        .arg("--socket")
        .arg(sock)
        .env_remove("XDG_RUNTIME_DIR")
        .env("HOME", home)
        .env("RUST_LOG", "info")
        .stdout(Stdio::null())
        .stderr(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .expect("log"),
        )
        .spawn()
        .expect("agentd")
}

pub struct Real {
    pub dir: tempfile::TempDir,
    pub sock: PathBuf,
    pub log: PathBuf,
    pub db: PathBuf,
    home: PathBuf,
    pub admin_token: String,
    pub chat_token: String,
    pub http: HttpClient,
    child: Option<Child>,
}

fn new_token(kind: TokenKind, scope: Scope) -> NewToken {
    NewToken {
        label: "s4-real".into(),
        kind,
        scope,
        ttl_days: None,
        allowlist: None,
        created_by: None,
    }
}

impl Real {
    pub async fn start() -> Real {
        let dir = tempfile::tempdir().expect("tmp");
        let db = dir.path().join("agentos/sessions.db");
        let sock = dir.path().join("agentd.sock");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let (admin, chat) = {
            let handle = Arc::new(Db::open(DbConfig::new(&db)).expect("db"));
            let store = TokenStore::new(handle, Arc::new(SystemClock), TokenPolicy::default());
            let a = store
                .issue(new_token(TokenKind::Gui, Scope::Admin))
                .await
                .expect("admin");
            let c = store
                .issue(new_token(TokenKind::External, Scope::Chat))
                .await
                .expect("chat");
            (a.value.expose().clone(), c.value.expose().clone())
        };
        let log = dir.path().join("agentd.log");
        let child = spawn(&db, &sock, &home, &log);
        let http = HttpClient::new(
            Endpoint::Unix(sock.clone()),
            Some(Token::new(admin.clone())),
        )
        .with_timeout(WAIT);
        let mut real = Real {
            dir,
            sock,
            log,
            db,
            home,
            admin_token: admin,
            chat_token: chat,
            http,
            child: Some(child),
        };
        real.wait_ready().await;
        real
    }

    async fn wait_ready(&mut self) {
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            if let Ok(r) = self.http.send(Request::get("/v1/health")).await
                && String::from_utf8_lossy(&r.body).contains("\"status\":\"ok\"")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("agentd did not become ready within {WAIT:?}");
    }

    /// Core crash (`SIGKILL`) and a new start on the same DB.
    pub async fn crash_and_restart(&mut self) {
        if let Some(mut c) = self.child.take() {
            drop(c.kill());
            drop(c.wait());
        }
        drop(std::fs::remove_file(&self.sock));
        self.child = Some(spawn(&self.db, &self.sock, &self.home, &self.log));
        self.wait_ready().await;
    }

    pub fn endpoint(&self) -> Endpoint {
        Endpoint::Unix(self.sock.clone())
    }

    pub fn client(&self) -> CoreClient {
        let mut c = CoreConfig::new(
            self.endpoint(),
            TokenSource::Value(Token::new(self.admin_token.clone())),
        );
        c.reconnect = ReconnectPolicy {
            base: Duration::from_millis(20),
            max: Duration::from_millis(100),
            jitter: 0.2,
        };
        c.ping_interval = Duration::from_secs(60);
        c.request_timeout = Duration::from_secs(5);
        c.sync_timeout = Duration::from_millis(500);
        CoreClient::start(c).expect("client")
    }

    /// `(status, json)` of a raw request on behalf of `admin`.
    pub async fn raw(&self, req: Request) -> (u16, Value) {
        let r = self.http.send(req).await.expect("request");
        (
            r.status,
            serde_json::from_slice(&r.body).unwrap_or(Value::Null),
        )
    }

    pub async fn post(&self, path: &str, body: Value) -> Value {
        let (st, v) = self.raw(Request::post(&format!("/v1{path}"), body)).await;
        assert!(st == 200 || st == 201, "POST {path}: {st} {v}");
        v
    }

    pub async fn patch(&self, path: &str, body: Value) -> Value {
        let (st, v) = self.raw(Request::patch(&format!("/v1{path}"), body)).await;
        assert_eq!(st, 200, "PATCH {path}: {v}");
        v
    }

    pub async fn get(&self, path: &str) -> Value {
        let (st, v) = self.raw(Request::get(&format!("/v1{path}"))).await;
        assert_eq!(st, 200, "GET {path}: {v}");
        v
    }

    /// The `scripted` account, the model on the `models` page, a session with that model: `session_id`.
    pub async fn model_session(&self) -> i64 {
        let acc = self
            .post("/accounts", json!({"type": "scripted", "label": "test"}))
            .await["id"]
            .as_i64()
            .expect("account id");
        let page = self.get("/settings/models").await;
        let rev = page["rev"].as_i64().unwrap_or(0);
        let (st, b) = self
            .raw(
                Request::patch(
                    "/v1/settings/models",
                    json!({"accounts": {acc.to_string(): {"model": "scripted"}}}),
                )
                .header("If-Match", &rev.to_string()),
            )
            .await;
        assert_eq!(st, 200, "{b}");
        let s = self.post("/sessions", json!({"name": "chat"})).await["id"]
            .as_i64()
            .expect("session id");
        self.patch(&format!("/sessions/{s}"), json!({"cfg": {"model": acc}}))
            .await;
        s
    }

    /// The action journal straight from the DB: the core has no `GET /actions` route (it is not in `openapi-draft.yaml`).
    /// Rows are `{id, session_id, action, decision, result, reversible}`, new ones on top.
    pub fn actions(&self) -> Vec<Value> {
        let c = rusqlite::Connection::open_with_flags(
            &self.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("db");
        let mut st = c
            .prepare("SELECT id, session_id, action, decision, result, reversible, args_json FROM actions_log ORDER BY id DESC")
            .expect("sql");
        st.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "session_id": r.get::<_, Option<i64>>(1)?,
                "action": r.get::<_, String>(2)?, "decision": r.get::<_, Option<String>>(3)?,
                "result": r.get::<_, Option<String>>(4)?, "reversible": r.get::<_, i64>(5)? != 0,
                "args_json": r.get::<_, Option<String>>(6)?,
            }))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
    }

    /// Findings of the observer straight from the DB: `{kind, verdict, ref_table}`.
    pub fn findings(&self, kind: &str) -> Vec<Value> {
        let c = rusqlite::Connection::open_with_flags(
            &self.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("db");
        let mut st = c
            .prepare("SELECT kind, verdict, ref_table FROM observer_findings WHERE kind = ?1 ORDER BY id DESC")
            .expect("sql");
        st.query_map([kind], |r| {
            Ok(json!({"kind": r.get::<_, String>(0)?, "verdict": r.get::<_, String>(1)?, "ref_table": r.get::<_, Option<String>>(2)?}))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Real {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            drop(c.kill());
            drop(c.wait());
        }
    }
}

pub fn frame_kind(ev: &CoreEvent) -> Option<&str> {
    match ev {
        CoreEvent::Frame(f) => f.data["kind"].as_str(),
        _ => None,
    }
}
