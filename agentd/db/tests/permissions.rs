//! A4-01 (DB): permission defaults and ignoring a relaxing row in the table.

use std::sync::Arc;

use agentd_core::{Action, Audit, Level, ManualClock, Mode, PermEngine, PutError, Rule, RuleStore};
use agentd_db::{Db, DbConfig, PermRepo};

async fn setup() -> (tempfile::TempDir, Arc<Db>, PermEngine, Arc<PermRepo>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1),(2,'b',1,1);
             INSERT INTO personas(id, name, color) VALUES (7,'p','#fff');",
        )
    })
    .await
    .unwrap();
    let repo = Arc::new(PermRepo::new(db.clone()));
    let audit = Audit::new(repo.clone(), Arc::new(ManualClock::new(1_800_000_000)));
    (dir, db, PermEngine::new(repo.clone(), audit), repo)
}

#[tokio::test]
async fn permissions_defaults_ask() {
    let (_d, _db, engine, repo) = setup().await;
    assert_eq!(repo.seed_defaults().await.unwrap(), 4);
    for a in [
        Action::Send,
        Action::Delete,
        Action::Pay,
        Action::ConfigWrite,
    ] {
        assert_eq!(
            engine.effective(None, None, a).await.unwrap(),
            Some(Mode::Ask),
            "{a}"
        );
    }
    // For the other actions the documents set no values: no row.
    for a in [
        Action::Read,
        Action::Shell,
        Action::Browse,
        Action::MemoryWrite,
    ] {
        assert_eq!(engine.effective(None, None, a).await.unwrap(), None, "{a}");
    }
    // Re-seeding writes nothing and does not overwrite what the owner changed.
    let allow = Rule {
        level: Level::Global,
        scope_id: 0,
        action: Action::Send,
        mode: Mode::Allow,
    };
    engine.put(allow, "test").await.unwrap();
    assert_eq!(repo.seed_defaults().await.unwrap(), 0);
    assert_eq!(
        engine.effective(None, None, Action::Send).await.unwrap(),
        Some(Mode::Allow)
    );
}

#[tokio::test]
async fn permissions_row_loosening_ignored() {
    let (_d, db, engine, _repo) = setup().await;
    // Manual DB edit bypassing the API: the session and persona relax a global deny.
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO permissions VALUES ('global',0,'delete','deny');
             INSERT INTO permissions VALUES ('session',1,'delete','allow');
             INSERT INTO permissions VALUES ('persona',7,'delete','ask');",
        )
    })
    .await
    .unwrap();
    assert_eq!(
        engine
            .effective(Some(7), Some(1), Action::Delete)
            .await
            .unwrap(),
        Some(Mode::Deny)
    );
    let found: Vec<(String, String, String, String)> = db
        .read(|c| {
            c.prepare("SELECT kind, verdict, ref_table, detail FROM observer_findings ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
        })
        .await
        .unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    for (kind, verdict, table, detail) in &found {
        assert_eq!(
            (kind.as_str(), verdict.as_str(), table.as_str()),
            ("permission_loosen", "blocked", "permissions")
        );
        assert!(detail.contains("\"action\":\"delete\""), "{detail}");
    }
    // The finding names the real level and number: persona 7 and session 1, not the other way around.
    let all = found
        .iter()
        .map(|f| f.3.clone())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        all.contains("\"level\":\"persona\"") && all.contains("\"scope_id\":7"),
        "{all}"
    );
    assert!(
        all.contains("\"level\":\"session\"") && all.contains("\"scope_id\":1"),
        "{all}"
    );
    // Re-reading the same row does not multiply findings.
    engine
        .effective(Some(7), Some(1), Action::Delete)
        .await
        .unwrap();
    let n: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM observer_findings", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn put_rule_rejects_loosening_and_unknown_scope() {
    let (_d, db, engine, _repo) = setup().await;
    let put = |level, id, action, mode| {
        let e = engine.clone();
        async move {
            e.put(
                Rule {
                    level,
                    scope_id: id,
                    action,
                    mode,
                },
                "gui",
            )
            .await
        }
    };
    put(Level::Global, 0, Action::Pay, Mode::Ask).await.unwrap();
    assert_eq!(
        put(Level::Session, 1, Action::Pay, Mode::Allow).await,
        Err(PutError::Loosen)
    );
    assert_eq!(
        put(Level::Persona, 7, Action::Pay, Mode::Allow).await,
        Err(PutError::Loosen)
    );
    put(Level::Session, 1, Action::Pay, Mode::Deny)
        .await
        .unwrap();
    assert_eq!(
        put(Level::Session, 99, Action::Pay, Mode::Deny).await,
        Err(PutError::NotFound)
    );
    assert_eq!(
        put(Level::Persona, 99, Action::Pay, Mode::Deny).await,
        Err(PutError::NotFound)
    );
    let rows: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM permissions WHERE scope IN ('session','persona') AND mode = 'allow'", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 0, "a relaxing row was not created");
    let findings: i64 = db
        .read(|c| {
            c.query_row(
                "SELECT count(*) FROM observer_findings WHERE kind='permission_loosen'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(findings, 2);
}
