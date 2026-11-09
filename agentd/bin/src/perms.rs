//! Assembly of permissions, journal and observer for `serve` (A4): defaults, permission requests, gate.

use std::path::PathBuf;
use std::sync::Arc;

use agentd_api::Hub;
use agentd_core::{
    Audit, BrowserBridge, ChatStore, Clock, Events, Observer, PermEngine, PermissionService,
    PolicyGate, RuleObserver, RuleStore, SettingsStore,
};
use agentd_db::{ChatRepo, Db, PermRepo, SettingsRepo};
use anyhow::Result;

/// Observer wrapper (model-based observer, A7-03).
pub(crate) type ObserverWrap = Box<dyn FnOnce(Arc<dyn Observer>) -> Arc<dyn Observer>>;

pub(crate) struct Perms {
    pub engine: PermEngine,
    pub gate: PolicyGate,
    pub observer: Arc<dyn Observer>,
    pub audit: Audit,
    pub browser: BrowserBridge,
}

/// Permission defaults (SPEC section 5) are seeded by first-run code, observer rules are read from
/// `~/.agentos/rules/*.toml` (a file error is a startup error), permission requests left open after a crash are closed with a denial,
/// the background expiry task is started.
pub(crate) async fn build(
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    chat: Arc<ChatRepo>,
    settings: Arc<SettingsRepo>,
    hub: Hub,
    events: Events,
    wrap_observer: Option<ObserverWrap>,
) -> Result<Perms> {
    let repo = Arc::new(PermRepo::new(db));
    let audit = Audit::new(repo.clone(), clock.clone()).with_events(events);
    repo.seed_defaults()
        .await
        .map_err(|e| anyhow::anyhow!("permission seeding: {e:?}"))?;
    let engine = PermEngine::new(repo, audit.clone());
    let chat: Arc<dyn ChatStore> = chat;
    let settings: Arc<dyn SettingsStore> = settings;
    let browser = BrowserBridge::new(Arc::new(hub.clone()), clock.clone());
    browser.spawn_sweeper();
    let requests = PermissionService::new(
        audit.clone(),
        chat,
        Arc::new(hub),
        clock,
        settings,
        engine.clone(),
    );
    requests
        .recover()
        .await
        .map_err(|e| anyhow::anyhow!("closing open permission requests: {e:?}"))?;
    requests.spawn_sweeper();
    let rules_dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".agentos/rules"));
    // `AGENTD_WATCH_SOCKET`: the rules are executed by a separate `agent-watch` process under its own uid; the core is only a client
    // (unreachable process means unreachable observer, dangerous actions are denied). Otherwise the core reads the rules itself.
    let mut observer: Arc<dyn Observer> = match crate::watch::socket() {
        Some(sock) => {
            let remote = Arc::new(agentd_core::RemoteObserver::new(
                sock,
                std::time::Duration::from_secs(2),
            ));
            remote.spawn_heartbeat(std::time::Duration::from_secs(10));
            remote
        }
        None => Arc::new(
            RuleObserver::load(rules_dir.as_deref())
                .map_err(|e| anyhow::anyhow!("observer rules: {e:?}"))?,
        ),
    };
    if let Some(wrap) = wrap_observer {
        observer = wrap(observer);
    }
    let gate = PolicyGate::new(engine.clone(), observer.clone(), audit.clone(), requests);
    Ok(Perms {
        engine,
        gate,
        observer,
        audit,
        browser,
    })
}
