//! Shared mock state: tokens, theme, request log, clock, broadcast.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::chat_state::ChatState;
use crate::clock::Clock;
use crate::config::CoreMode;
use crate::db::Db;
use crate::hub::Hub;
use crate::limiter::Limiter;
use crate::settings_store::SettingsStore;
use crate::store::{StoreError, TokenStore};
use crate::{EchoRecord, RequestRecord};

pub struct Inner {
    pub tokens: TokenStore,
    pub limiter: Limiter,
    /// The `settings` table (theme and its `rev`, db.md: `rev` grows on every write).
    pub settings: SettingsStore,
    pub echo: Vec<EchoRecord>,
    pub requests: Vec<RequestRecord>,
}

pub struct Shared {
    pub clock: Clock,
    /// Mock DB: the single writer.
    pub db: Arc<Db>,
    /// `index.db` and search.
    pub search: crate::search::SearchIndex,
    pub hub: Hub,
    pub chat: ChatState,
    /// Registry of event subtypes (`GET /events/subtypes`).
    pub registry: crate::events_registry::Registry,
    /// Open commands to the browser (`tool-results`).
    pub tools: crate::tools_state::ToolBridge,
    /// Mock data directory (next to the DB): `secrets.env`, `agentos/`.
    pub data_dir: std::path::PathBuf,
    /// Secrets (write-only, outside the tables).
    pub secrets: crate::secrets_http::SecretStore,
    /// Queue of failure injections (`/_test/fail`).
    pub fail: crate::fail_inject::Fails,
    pub api_major: u32,
    pub allowed_origins: Vec<String>,
    mode: Mutex<CoreMode>,
    tcp_port: AtomicU16,
    inner: Mutex<Inner>,
}

impl Shared {
    pub fn new(
        clock: Clock,
        api_major: u32,
        allowed_origins: Vec<String>,
        db: Arc<Db>,
        reply_delay: std::time::Duration,
        data_dir: std::path::PathBuf,
    ) -> Result<Shared, StoreError> {
        let settings = SettingsStore::new(db.clone(), clock.now_unix())?;
        let search = crate::search::SearchIndex::open(&data_dir)?;
        Ok(Shared {
            search,
            clock,
            db: db.clone(),
            hub: Hub::new(),
            chat: ChatState::new(reply_delay),
            registry: crate::events_registry::Registry::new(),
            tools: crate::tools_state::ToolBridge::default(),
            secrets: crate::secrets_http::SecretStore::new(&data_dir),
            fail: crate::fail_inject::Fails::default(),
            data_dir,
            api_major,
            allowed_origins,
            mode: Mutex::new(CoreMode::Ok),
            tcp_port: AtomicU16::new(0),
            inner: Mutex::new(Inner {
                tokens: TokenStore::new(db),
                limiter: Limiter::default(),
                settings,
                echo: Vec::new(),
                requests: Vec::new(),
            }),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn mode(&self) -> CoreMode {
        *self.mode.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_mode(&self, mode: CoreMode) {
        *self.mode.lock().unwrap_or_else(PoisonError::into_inner) = mode;
    }

    pub fn tcp_port(&self) -> u16 {
        self.tcp_port.load(Ordering::SeqCst)
    }

    pub fn set_tcp_port(&self, port: u16) {
        self.tcp_port.store(port, Ordering::SeqCst);
    }
}
