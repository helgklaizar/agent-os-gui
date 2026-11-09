//! A-core-100 (A4-13 item 3): permission engine rule cache with invalidation on write and an entry lifetime.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentd_core::{
    Action, Audit, Level, ManualClock, Mode, PermEngine, PermResult, PortFuture, Rule, RuleSet,
    RuleStore,
};
use agentd_db::{Db, DbConfig, PermRepo};

/// Counts accesses to the rule store.
struct Counting {
    inner: Arc<PermRepo>,
    reads: AtomicUsize,
}

impl RuleStore for Counting {
    fn rule_set(
        &self,
        persona_id: Option<i64>,
        session_id: Option<i64>,
        action: Action,
    ) -> PortFuture<'_, PermResult<RuleSet>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.rule_set(persona_id, session_id, action)
    }
    fn list_rules(
        &self,
        level: Option<Level>,
        scope_id: Option<i64>,
    ) -> PortFuture<'_, PermResult<Vec<Rule>>> {
        self.inner.list_rules(level, scope_id)
    }
    fn put_rule(&self, rule: Rule) -> PortFuture<'_, PermResult<()>> {
        self.inner.put_rule(rule)
    }
    fn session_persona(&self, session_id: i64) -> PortFuture<'_, PermResult<Option<i64>>> {
        self.inner.session_persona(session_id)
    }
    fn scope_exists(&self, level: Level, scope_id: i64) -> PortFuture<'_, PermResult<bool>> {
        self.inner.scope_exists(level, scope_id)
    }
    fn seed_defaults(&self) -> PortFuture<'_, PermResult<usize>> {
        self.inner.seed_defaults()
    }
}

async fn setup() -> (tempfile::TempDir, Arc<Db>, Arc<Counting>, PermEngine) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1);",
        )
    })
    .await
    .unwrap();
    let repo = Arc::new(PermRepo::new(db.clone()));
    repo.seed_defaults().await.unwrap();
    let store = Arc::new(Counting {
        inner: repo.clone(),
        reads: AtomicUsize::new(0),
    });
    let audit = Audit::new(repo, Arc::new(ManualClock::new(1_800_000_000)));
    let engine = PermEngine::new(store.clone(), audit);
    (dir, db, store, engine)
}

fn allow_send() -> Rule {
    Rule {
        level: Level::Global,
        scope_id: 0,
        action: Action::Send,
        mode: Mode::Allow,
    }
}

#[tokio::test]
async fn repeated_effective_hits_the_cache() {
    let (_d, _db, store, engine) = setup().await;
    for _ in 0..5 {
        assert_eq!(
            engine.effective(None, Some(1), Action::Send).await.unwrap(),
            Some(Mode::Ask)
        );
    }
    assert_eq!(
        store.reads.load(Ordering::SeqCst),
        1,
        "four of five come from the cache"
    );
    // A different action is a separate entry.
    engine.effective(None, Some(1), Action::Pay).await.unwrap();
    assert_eq!(store.reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn put_invalidates_the_cache() {
    let (_d, _db, store, engine) = setup().await;
    assert_eq!(
        engine.effective(None, Some(1), Action::Send).await.unwrap(),
        Some(Mode::Ask)
    );
    // `allow` over a global `ask` would relax the session; use the same level: tighten and relax the global rule.
    engine.put(allow_send(), "test").await.unwrap();
    assert_eq!(
        engine.effective(None, Some(1), Action::Send).await.unwrap(),
        Some(Mode::Allow),
        "after a rule write: fresh value, not the stale cached one"
    );
    assert!(store.reads.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn cache_entry_expires_after_ttl() {
    let (_d, db, store, engine) = setup().await;
    let engine = engine.with_cache_ttl(Duration::from_millis(60));
    assert_eq!(
        engine.effective(None, None, Action::Send).await.unwrap(),
        Some(Mode::Ask)
    );
    // DB edit bypassing the engine (manual, cascade): visible no later than the entry lifetime.
    db.write(|tx| {
        tx.execute(
            "UPDATE permissions SET mode = 'deny' WHERE action = 'send'",
            [],
        )
    })
    .await
    .unwrap();
    assert_eq!(
        engine.effective(None, None, Action::Send).await.unwrap(),
        Some(Mode::Ask),
        "before expiry: from the cache"
    );
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        engine.effective(None, None, Action::Send).await.unwrap(),
        Some(Mode::Deny)
    );
    assert!(store.reads.load(Ordering::SeqCst) >= 2);
}
