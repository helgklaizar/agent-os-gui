//! Helpers for `serve`: signals, clocks, TCP setup, migrations, the token issuance service socket.

use std::path::Path;
use std::sync::Arc;

use agentd_auth::{TokenPolicy, TokenStore};
use agentd_core::{SettingsStore, SystemClock};
use agentd_db::{Db, DbConfig, MIGRATIONS, SettingsRepo, check_migrations};
use anyhow::{Context, Result};

use crate::cli::Paths;

/// SIGTERM and SIGINT handlers are installed at the start of `serve`: the socket may open already during migration, and a signal
/// arriving earlier must not kill the process by default.
pub(super) struct Signals {
    term: tokio::signal::unix::Signal,
    int: tokio::signal::unix::Signal,
}

impl Signals {
    pub(super) fn install() -> Result<Signals> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Signals {
            term: signal(SignalKind::terminate()).context("SIGTERM subscription")?,
            int: signal(SignalKind::interrupt()).context("SIGINT subscription")?,
        })
    }

    pub(super) async fn wait(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.int.recv() => {}
        }
    }
}

/// Clocks and token policy. Real clocks always, except in a `test-support` build with the `AGENTD_TEST_CLOCK` variable:
/// then the clocks are controllable (`POST /v1/_test/advance`) and the `test` token kind is allowed. In a release build
/// `agentd_core::manual_clock` returns `None`, and the variable changes nothing.
pub(super) fn build_clock_and_policy() -> (Arc<dyn agentd_core::Clock>, TokenPolicy) {
    let now = agentd_core::Clock::now_unix(&SystemClock);
    if std::env::var_os("AGENTD_TEST_CLOCK").is_some()
        && let Some(clock) = agentd_core::manual_clock(now)
    {
        let policy = TokenPolicy {
            allow_test_kind: true,
            ..TokenPolicy::default()
        };
        return (clock, policy);
    }
    (Arc::new(SystemClock), TokenPolicy::default())
}

/// Value of the `tunnels.api_external` setting at start; page not written means off (TCP closed by default).
pub(super) async fn tcp_enabled(settings: &SettingsRepo) -> Result<bool> {
    let page = settings
        .get("tunnels")
        .await
        .map_err(|e| anyhow::anyhow!("reading the tunnels page: {e:?}"))?;
    Ok(page
        .doc
        .get("api_external")
        .and_then(serde_json::Value::as_bool)
        == Some(true))
}

/// `migrate`: apply migrations; `--check`: only report. `--check` codes: 0, 1, 3 (see USAGE).
pub fn migrate(db: &Path, check: bool) -> Result<u8> {
    if check {
        let c = check_migrations(db, MIGRATIONS).context("DB version check")?;
        println!("user_version {} of {}", c.db_version, c.known_version);
        return Ok(if c.newer_than_code() {
            3
        } else {
            u8::from(c.pending())
        });
    }
    drop(Db::open(DbConfig::new(db)).with_context(|| format!("DB {}", db.display()))?);
    let c = check_migrations(db, MIGRATIONS).context("DB version check")?;
    println!("user_version {}", c.db_version);
    Ok(0)
}

/// Service socket for GUI token issuance; Linux only (needs `/proc` and `SCM_RIGHTS`).
pub(super) fn start_launch(
    p: &Paths,
    store: &Arc<TokenStore>,
    hub: agentd_api::Hub,
    audit: agentd_core::Audit,
    sessions: Arc<dyn crate::launch::SessionLookup>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    if !cfg!(target_os = "linux") {
        tracing::warn!(
            scope = "auth",
            event = "launch_unsupported",
            "GUI token issuance is available only on Linux"
        );
        return Ok(None);
    }
    if agentd_sys::proc::ptrace_scope() != Some(1) {
        tracing::warn!(
            scope = "auth",
            event = "ptrace_scope",
            "kernel.yama.ptrace_scope is not 1: the GUI token scheme is weaker (SP-1.5)"
        );
    }
    let policy = crate::launch::LaunchPolicy {
        ancestor: p.launch_ancestor.clone(),
        service: p.launch_service.clone(),
        agent_ancestor: p.agent_ancestor.clone(),
        agent_unit: p.agent_unit.clone(),
    };
    let listener = crate::launch::bind(&p.launch_socket)?;
    let svc = crate::launch::LaunchService::new(store.clone(), hub, policy, audit, sessions);
    Ok(Some(crate::launch::spawn(listener, svc)))
}
