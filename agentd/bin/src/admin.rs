//! `agentd token issue|list|revoke`: tokens for external clients (browser client, phone) from the command line.
//! With a live core the command goes through the service socket `agentd-admin.sock` (`0600`, same uid), with a stopped one,
//! straight to the DB. The token value is printed once and stored nowhere (only the hash in the DB).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentd_api::Hub;
use agentd_auth::TokenStore;
use agentd_core::{
    ExternalTokenRequest, RevokeOutcome, Scope, SystemClock, TokenAdmin, TokenAdminError,
};
use agentd_db::{Db, DbConfig};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, PartialEq, Eq)]
pub enum TokenOp {
    Issue {
        name: String,
        scope: Scope,
        ttl_days: Option<u32>,
        /// Web token allowed to decide permission requests (ADR-0010 item 4): `chat` only, lifetime up to 30 days.
        decide: bool,
    },
    List,
    Revoke {
        id: i64,
    },
}

impl TokenOp {
    fn to_request(&self) -> Value {
        match self {
            TokenOp::Issue {
                name,
                scope,
                ttl_days,
                decide,
            } => {
                json!({"op": "issue", "name": name, "scope": scope.as_str(), "ttl_days": ttl_days, "decide": decide})
            }
            TokenOp::List => json!({"op": "list"}),
            TokenOp::Revoke { id } => json!({"op": "revoke", "id": id}),
        }
    }
}

fn fail(code: &str) -> Value {
    json!({"ok": false, "error": code})
}

fn admin_error(e: &TokenAdminError) -> Value {
    match e {
        TokenAdminError::Invalid(f) => {
            let first = f.first().map_or("invalid", |x| x.field.as_str());
            json!({"ok": false, "error": format!("invalid:{first}")})
        }
        TokenAdminError::Unavailable => fail("unavailable"),
    }
}

/// Executes a request against the token store; `hub` closes the WS of a revoked token (no core means `None`).
pub async fn handle(store: &TokenStore, hub: Option<&Hub>, req: &Value) -> Value {
    match req["op"].as_str() {
        Some("issue") => {
            let Some(scope) = req["scope"].as_str().and_then(Scope::parse) else {
                return fail("invalid:scope");
            };
            let r = ExternalTokenRequest {
                name: req["name"].as_str().unwrap_or_default().to_string(),
                scope,
                ttl_days: req["ttl_days"].as_u64().and_then(|n| u32::try_from(n).ok()),
                created_by: None,
            };
            let issued = if req["decide"] == true {
                store.issue_web(r).await
            } else {
                store.issue_external(r).await
            };
            match issued {
                Ok(i) => json!({"ok": true, "id": i.info.id, "token": i.value.expose()}),
                Err(e) => admin_error(&e),
            }
        }
        Some("list") => match store.list().await {
            Ok(items) => {
                let rows: Vec<Value> = items
                    .iter()
                    .map(|t| {
                        json!({"id": t.id, "name": t.name, "kind": t.kind.as_str(), "scope": t.scope.as_str(),
                               "created_at": t.created_at, "expires_at": t.expires_at, "revoked": t.revoked})
                    })
                    .collect();
                json!({"ok": true, "items": rows})
            }
            Err(e) => admin_error(&e),
        },
        Some("revoke") => {
            let Some(id) = req["id"].as_i64() else {
                return fail("invalid:id");
            };
            match TokenAdmin::revoke(store, id).await {
                Ok(RevokeOutcome::Revoked) => {
                    if let Some(h) = hub {
                        h.revoke_token(id);
                    }
                    json!({"ok": true})
                }
                Ok(RevokeOutcome::AlreadyRevoked) => json!({"ok": true}),
                Ok(RevokeOutcome::NotFound) => fail("not_found"),
                Err(e) => admin_error(&e),
            }
        }
        _ => fail("unknown_op"),
    }
}

async fn serve_conn(store: Arc<TokenStore>, hub: Hub, stream: UnixStream) {
    let same_uid = stream
        .peer_cred()
        .is_ok_and(|c| c.uid() == agentd_sys::current_uid());
    let (rd, mut wr) = stream.into_split();
    let mut line = String::new();
    let reply = if same_uid
        && matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                BufReader::new(rd).read_line(&mut line)
            )
            .await,
            Ok(Ok(n)) if n > 0
        ) {
        match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&store, Some(&hub), &req).await,
            Err(_) => fail("bad_request"),
        }
    } else {
        tracing::warn!(
            scope = "auth",
            event = "admin_socket_denied",
            "service socket: denied"
        );
        fail("denied")
    };
    if wr.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
        tracing::debug!(scope = "auth", event = "admin_reply_failed", "client gone");
    }
}

/// Service socket listener: created with `0600` (`launch::bind`).
pub fn spawn(path: &Path, store: Arc<TokenStore>, hub: Hub) -> Result<tokio::task::JoinHandle<()>> {
    let listener: UnixListener = crate::launch::bind(path)?;
    Ok(tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((s, _)) => {
                    tokio::spawn(serve_conn(store.clone(), hub.clone(), s));
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    }))
}

async fn ask(socket: &Path, req: &Value) -> Result<Value> {
    let mut s = UnixStream::connect(socket).await?;
    s.write_all(format!("{req}\n").as_bytes()).await?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).await?;
    serde_json::from_str(&line).context("core reply not parsed")
}

fn print_reply(op: &TokenOp, reply: &Value) -> Result<u8> {
    if reply["ok"] != true {
        eprintln!("agentd: {}", reply["error"].as_str().unwrap_or("error"));
        return Ok(1);
    }
    match op {
        TokenOp::Issue { .. } => println!("{}", reply["token"].as_str().unwrap_or_default()),
        TokenOp::List => {
            for t in reply["items"].as_array().into_iter().flatten() {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    t["id"],
                    t["name"].as_str().unwrap_or_default(),
                    t["kind"].as_str().unwrap_or_default(),
                    t["scope"].as_str().unwrap_or_default(),
                    t["expires_at"]
                        .as_i64()
                        .map_or("-".to_string(), |v| v.to_string()),
                    if t["revoked"] == true {
                        "revoked"
                    } else {
                        "active"
                    },
                );
            }
        }
        TokenOp::Revoke { .. } => {}
    }
    Ok(0)
}

/// The `token` subcommand: the service socket of a live core, otherwise the DB directly.
pub async fn run(op: TokenOp, admin_socket: PathBuf, db: PathBuf) -> Result<u8> {
    let req = op.to_request();
    if let Ok(reply) = ask(&admin_socket, &req).await {
        return print_reply(&op, &reply);
    }
    let handle_db = tokio::task::spawn_blocking(move || Db::open(DbConfig::new(&db)))
        .await
        .context("DB open thread")?
        .context("DB not opened: the core is running without a service socket or the DB is busy")?;
    let store = TokenStore::new(
        Arc::new(handle_db),
        Arc::new(SystemClock),
        agentd_auth::TokenPolicy::default(),
    );
    print_reply(&op, &handle(&store, None, &req).await)
}
