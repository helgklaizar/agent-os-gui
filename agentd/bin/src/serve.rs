//! The `serve` and `migrate` subcommands.

use std::sync::Arc;

use agentd_api::{ServerConfig, Services};
use agentd_auth::TokenStore;
use agentd_db::{
    ChatRepo, Db, DbConfig, EventRepo, ExportRepo, FileRepo, MemoryRepo, ModuleRepo, QuotaRepo,
    SearchRepo, SettingsRepo, TaskRepo, TreeRepo, TriggerRepo, TunnelRepo,
};
use agentd_providers::KeysFile;
use anyhow::{Context, Result};

mod background;
mod ops;
mod support;
mod watchdog;

use crate::boot::Boot;
use crate::cli::Paths;

pub async fn serve(p: Paths) -> Result<()> {
    let mut signals = support::Signals::install()?;
    let mut cfg = ServerConfig::new(&p.socket);
    let (clock, policy) = support::build_clock_and_policy();
    cfg.clock = clock.clone();
    if let Some(addr) = p.tcp {
        cfg.tcp_addr = addr;
        // An explicit `--tcp` is a deliberate opt-in: the `tunnels.api_external` setting does not turn it off.
        cfg.tcp_enabled = true;
    }
    p.web.apply(&mut cfg);
    let boot = Boot::new(cfg, tokio::runtime::Handle::current());
    let mut db_cfg = DbConfig::new(&p.db);
    let hook = boot.clone();
    db_cfg.migrate.on_event = Some(Arc::new(move |e| hook.on_event(e)));
    // DB and migrations come before the socket: a DB newer than the code or a second instance is refused without an open port. If migrations are running,
    // the runner opens the socket (the `Started` event) and `/health` shows `migrating`.
    let opened = tokio::task::spawn_blocking(move || Db::open(db_cfg))
        .await
        .context("DB open thread")?;
    let db = match opened {
        Ok(db) => Arc::new(db),
        Err(e) => {
            if let Some(server) = boot.take_started() {
                // Migration failed with the socket open: until the stop signal `/health` returns `failed` (docs/reference/api.md section 2.1).
                tracing::error!(
                    scope = "migrator",
                    event = "migration_failed",
                    "migration failed, the core answers only on /health"
                );
                signals.wait().await;
                server.shutdown().await;
            }
            return Err(anyhow::Error::new(e).context(format!("DB {}", p.db.display())));
        }
    };
    let store = Arc::new(TokenStore::new(db.clone(), clock.clone(), policy));
    let settings = Arc::new(SettingsRepo::new(db.clone(), clock.clone()));
    let tcp_on = support::tcp_enabled(settings.as_ref()).await?;
    let server = boot.server().await?;
    let store_for_launch = store.clone();
    // The home directory is the symlink check boundary for a session `path` (A2-05).
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    // Secrets live in `~/.agentos/keys.env`; when an account is removed its secret is erased (A3-02).
    let keys = Arc::new(KeysFile::new(
        home.as_deref()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".agentos/keys.env"),
    ));
    let agentos = home
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(".agentos");
    let chat = Arc::new(ChatRepo::new(db.clone(), clock.clone(), agentos.clone()));
    let tree = Arc::new(
        TreeRepo::new(db.clone(), clock.clone(), Arc::new(server.hub()), home)
            .with_secret_eraser(keys.clone()),
    );
    let quotas = Arc::new(QuotaRepo::new(db.clone(), clock.clone()));
    let event_repo = Arc::new(EventRepo::new(
        db.clone(),
        clock.clone(),
        Arc::new(server.hub()),
    ));
    let events = agentd_core::Events::new(event_repo.clone());
    let providers = agentd_providers::standard_registry(
        keys.clone(),
        clock.clone(),
        agentd_providers::HttpClient::new(agentd_providers::Timeouts::default()),
    );
    // Model-based observer (A7-03): with `AGENTD_WATCH=1` it wraps the rules; otherwise the rules stand alone.
    let wrap_observer: Option<crate::perms::ObserverWrap> = crate::watch::enabled().then(|| {
        let deps = crate::watch::Deps {
            providers: providers.clone(),
            tree: tree.clone(),
            chat: chat.clone(),
            settings: settings.clone(),
        };
        Box::new(move |rules| crate::watch::wrap(rules, deps)) as Box<_>
    });
    let perms = crate::perms::build(
        db.clone(),
        clock.clone(),
        chat.clone(),
        settings.clone(),
        server.hub(),
        events.clone(),
        wrap_observer,
    )
    .await?;
    let sys = agentd_core::SysSnapshot::default();
    background::spawn_sys(sys.clone(), server.hub());
    let stack = crate::stack::build(crate::stack::Deps {
        db: &db,
        clock: &clock,
        hub: server.hub(),
        settings: settings.clone(),
        chat: chat.clone(),
        tree: tree.clone(),
        quotas: quotas.clone(),
        events_repo: event_repo.clone(),
        events: events.clone(),
        keys: keys.clone(),
        perms: &perms,
        sys: sys.clone(),
        agentos: agentos.clone(),
    });
    let mcp = agentd_core::Mcp::new(
        settings.clone(),
        Arc::new(
            agentd_providers::RmcpConnector::from_env().map_err(|e| anyhow::anyhow!("MCP: {e}"))?,
        ),
        Arc::new(server.hub()),
    );
    mcp.spawn(std::time::Duration::from_secs(3));
    let runner = Arc::new(crate::runner_client::RunnerClient::from_env());
    let responder = agentd_core::ChatService::new(
        chat.clone(),
        tree.clone(),
        settings.clone(),
        providers,
        Some(quotas.clone()),
        Arc::new(server.hub()),
        agentd_core::ChatConfig::default(),
    )
    .with_events(events.clone())
    .with_actions(
        perms.gate.clone(),
        agentd_core::Executors::standard(settings.clone(), Arc::new(server.hub()))
            .with(
                agentd_core::Action::Browse,
                Arc::new(agentd_core::BrowseExecutor::new(perms.browser.clone())),
            )
            .with(
                agentd_core::Action::Shell,
                crate::shell_tokens::executor(
                    runner.clone(),
                    tree.clone(),
                    (&store_for_launch, server.hub(), perms.audit.clone()),
                ),
            )
            .with(
                agentd_core::Action::MemoryWrite,
                Arc::new(agentd_core::MemoryExecutor::new(
                    Arc::new(MemoryRepo::new(db.clone())),
                    clock.clone(),
                )),
            )
            .with(
                agentd_core::Action::Send,
                Arc::new(agentd_core::ChannelSend::new(
                    stack.channels.clone(),
                    stack.gateway.clone(),
                    clock.clone(),
                )),
            ),
    )
    .with_mcp(mcp.clone());
    let tasks = agentd_core::Tasks::new(
        Arc::new(TaskRepo::new(db.clone(), clock.clone())),
        clock.clone(),
        Arc::new(server.hub()),
        events.clone(),
        settings.clone(),
        responder.clone(),
    );
    tasks.spawn(std::time::Duration::from_secs(1));
    let triggers = agentd_core::Triggers::new(
        Arc::new(TriggerRepo::new(db.clone())),
        clock.clone(),
        responder.clone(),
        stack.gateway.clone(),
        events.clone(),
    );
    stack.gateway.set_hook(Arc::new(triggers.clone()));
    triggers.set_tasks(tasks.clone());
    mcp.set_notice_hook(Arc::new(triggers.clone()));
    // Core start: replies interrupted by a crash are moved to `interrupted` before requests are accepted.
    responder
        .recover()
        .await
        .map_err(|e| anyhow::anyhow!("moving streaming to interrupted: {e:?}"))?;
    let mut services = Services::new(store.clone());
    services.channel_control = stack.control.clone();
    services.channels = Some(stack.channels);
    services.gateway = Some(stack.gateway);
    services.indicators = Some(stack.indicators);
    services.tokens = Some(store);
    services.settings = Some(settings.clone());
    services.status = Some(db.clone());
    let sessions_for_launch: Arc<dyn crate::launch::SessionLookup> = tree.clone();
    services.tree = Some(tree.clone());
    services.secrets = Some(keys);
    services.chat = Some(chat);
    services.responder = Some(responder);
    background::spawn_retention(db.clone(), clock.clone(), events.clone(), &agentos);
    crate::dbus_notify::spawn(events.clone(), clock.clone());
    services.tasks = Some(tasks);
    services.memory_mcp = Some(agentd_core::McpMemorySource::new(mcp));
    services.exporter = Some(Arc::new(ExportRepo::new(db.clone(), &agentos)));
    let modules = Arc::new(ModuleRepo::new(db.clone(), &agentos));
    let tunnels = Arc::new(TunnelRepo::new(db.clone()));
    crate::json_export::spawn(
        agentos.clone(),
        crate::json_export::Sources {
            settings: settings.clone(),
            tree: tree.clone(),
            modules: modules.clone(),
            tunnels: tunnels.clone(),
        },
        std::time::Duration::from_secs(2),
    );
    let tunnel_statuses = agentd_core::TunnelStatuses::default();
    crate::tunnel_proc::spawn(crate::tunnel_proc::Runner {
        store: tunnels.clone(),
        statuses: tunnel_statuses.clone(),
        events: events.clone(),
        audit: perms.audit.clone(),
    });
    services.tunnel_statuses = Some(tunnel_statuses);
    let weather = agentd_core::WeatherSnapshot::default();
    background::spawn_weather(
        weather.clone(),
        settings.clone(),
        clock.clone(),
        server.hub(),
    );
    services.weather = Some(weather);
    services.sys = Some(sys);
    services.logout = Some(Arc::new(crate::logout::Loginctl));
    services.module_fetcher = Some(Arc::new(agentd_providers::HttpModuleFetcher::new(
        agentd_providers::HttpClient::new(agentd_providers::Timeouts::default()),
    )));
    services.modules = Some(modules);
    services.tunnels = Some(tunnels);
    services.files = Some(Arc::new(FileRepo::new(agentos.clone())));
    services.search = match SearchRepo::open(db.clone(), &agentos) {
        Ok(repo) => Some(Arc::new(repo)),
        Err(e) => {
            tracing::warn!(scope = "search", event = "index_open_failed", error = %e, "search index not opened");
            None
        }
    };
    services.triggers = Some(triggers);
    services.quotas = Some(quotas);
    services.events = Some(event_repo);
    services.engine = Some(perms.engine);
    services.gate = Some(perms.gate);
    services.observer = Some(perms.observer);
    let audit_for_launch = perms.audit.clone();
    services.audit = Some(perms.audit);
    services.browser = Some(perms.browser);
    // The `agentd token` service socket is opened before readiness: `GET /v1/health` = `ok` means it already accepts requests.
    let admin_task = crate::admin::spawn(&p.admin_socket, store_for_launch.clone(), server.hub())?;
    server.set_ready(services);
    background::spawn_capabilities(server.health(), runner.socket().to_path_buf());
    server.tcp().set_enabled(tcp_on || p.tcp.is_some());
    let launch_task = support::start_launch(
        &p,
        &store_for_launch,
        server.hub(),
        audit_for_launch,
        sessions_for_launch,
    )?;
    tracing::info!(scope = "api", event = "started", "agentd started");
    let ops = ops::Ops::start();
    signals.wait().await;
    tracing::info!(scope = "api", event = "stopping", "stopping");
    ops.stop();
    admin_task.abort();
    drop(std::fs::remove_file(&p.admin_socket));
    if let Some(t) = launch_task {
        t.abort();
    }
    server.shutdown().await;
    Ok(())
}

pub use support::migrate;
