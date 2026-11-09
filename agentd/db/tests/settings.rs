//! A1-09: settings pages in the DB: `rev`, merge, validation, surviving a restart.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use agentd_core::{Clock, SettingsError, SettingsStore};
use agentd_db::{Db, DbConfig, SettingsRepo};
use serde_json::json;

#[derive(Debug)]
struct Fixed(AtomicI64);

impl Clock for Fixed {
    fn now_unix(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.load(Ordering::SeqCst)).unwrap_or(0) * 1000
    }
}

fn open(dir: &std::path::Path, now: i64) -> (Arc<Db>, SettingsRepo) {
    let db = Arc::new(Db::open(DbConfig::new(dir.join("agentos/sessions.db"))).unwrap());
    let repo = SettingsRepo::new(db.clone(), Arc::new(Fixed(AtomicI64::new(now))));
    (db, repo)
}

#[tokio::test]
async fn absent_page_is_default_with_rev_zero() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, repo) = open(dir.path(), 1_800_000_000);
    let t = repo.get("theme").await.unwrap();
    assert_eq!((t.rev, t.doc), (0, json!({"theme": "auto"})));
    let t = repo.get("tunnels").await.unwrap();
    assert_eq!(t.doc["api_external"], false);
    assert!(matches!(
        repo.get("nope").await,
        Err(SettingsError::UnknownPage)
    ));
    assert!(
        repo.revisions().await.unwrap().is_empty(),
        "nothing written"
    );
}

#[tokio::test]
async fn patch_merges_bumps_rev_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let (db, repo) = open(dir.path(), 1_800_000_000);
    let a = repo
        .patch(
            "schedule",
            json!({"summary": {"time": "08:30", "enabled": true}}),
            0,
        )
        .await
        .unwrap();
    assert_eq!(a.rev, 1);
    let b = repo
        .patch(
            "schedule",
            json!({"summary": {"enabled": null}, "backup": {"time": "03:00"}}),
            1,
        )
        .await
        .unwrap();
    assert_eq!(b.rev, 2);
    assert_eq!(
        b.doc,
        json!({"summary": {"time": "08:30"}, "backup": {"time": "03:00"}})
    );
    let (rev, at): (i64, i64) = db
        .read(|c| {
            c.query_row(
                "SELECT rev, updated_at FROM settings WHERE key='schedule'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        (rev, at),
        (2, 1_800_000_000),
        "rev and updated_at in the table"
    );
    drop(repo);
    drop(db);
    let (_db, repo) = open(dir.path(), 1_900_000_000);
    let again = repo.get("schedule").await.unwrap();
    assert_eq!(
        (again.rev, again.doc),
        (
            2,
            json!({"summary": {"time": "08:30"}, "backup": {"time": "03:00"}})
        )
    );
    assert_eq!(repo.revisions().await.unwrap().get("schedule"), Some(&2));
}

#[tokio::test]
async fn stale_if_match_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, repo) = open(dir.path(), 1);
    repo.patch("memory", json!({"source": "both"}), 0)
        .await
        .unwrap();
    for stale in [0, 5] {
        match repo
            .patch("memory", json!({"source": "local"}), stale)
            .await
        {
            Err(SettingsError::Stale { current }) => assert_eq!(current, 1),
            other => panic!("expected Stale, got {other:?}"),
        }
    }
    assert_eq!(repo.get("memory").await.unwrap().doc["source"], "both");
}

#[tokio::test]
async fn invalid_and_read_only_are_rejected_without_write() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, repo) = open(dir.path(), 1);
    match repo.patch("theme", json!({"theme": "red"}), 0).await {
        Err(SettingsError::Invalid(f)) => assert_eq!(f[0].field, "theme"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        repo.patch("about", json!({"x": 1}), 0).await,
        Err(SettingsError::ReadOnly)
    ));
    assert!(matches!(
        repo.patch("nope", json!({}), 0).await,
        Err(SettingsError::UnknownPage)
    ));
    assert_eq!(
        repo.get("theme").await.unwrap().rev,
        0,
        "no record after a refusal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_patches_with_same_if_match_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, repo) = open(dir.path(), 1);
    let repo = Arc::new(repo);
    let mut tasks = Vec::new();
    for i in 0..20 {
        let r = repo.clone();
        tasks.push(tokio::spawn(async move {
            r.patch("account", json!({"lat": i}), 0).await
        }));
    }
    let mut ok = 0;
    let mut stale = 0;
    for t in tasks {
        match t.await.unwrap() {
            Ok(d) => {
                assert_eq!(d.rev, 1);
                ok += 1;
            }
            Err(SettingsError::Stale { .. }) => stale += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!((ok, stale), (1, 19), "one write wins, the rest get 412");
}

async fn seed_legacy(db: &Db) {
    db.write(|tx| {
        tx.execute(
            "INSERT INTO settings(key, value_json, rev, updated_at) VALUES \
             ('account', '{\"display_name\":\"Owner\",\"extra\":{\"deep\":1}}', 1, 1)",
            [],
        )
    })
    .await
    .unwrap();
}

/// A document written before schemas were introduced contains extra fields: they are allowed while already in the document,
/// but a new `PATCH` cannot add an extra field.
#[tokio::test]
async fn legacy_extra_fields_do_not_block_patch_but_cannot_be_added() {
    let dir = tempfile::tempdir().unwrap();
    let (db, repo) = open(dir.path(), 1);
    seed_legacy(&db).await;
    let ok = repo
        .patch("account", json!({"name": "New"}), 1)
        .await
        .unwrap();
    assert_eq!(ok.rev, 2, "extra fields of the old document do not block");
    assert_eq!(ok.doc["display_name"], "Owner");
    assert_eq!(ok.doc["extra"]["deep"], 1);
    let ok = repo
        .patch("account", json!({"display_name": "Other"}), 2)
        .await
        .unwrap();
    assert_eq!(
        ok.doc["display_name"], "Other",
        "an already existing field can be changed"
    );
    match repo
        .patch("account", json!({"brand_new": 1, "extra": {"more": 2}}), 3)
        .await
    {
        Err(SettingsError::Invalid(f)) => {
            let names: Vec<String> = f.iter().map(|e| e.field.clone()).collect();
            assert!(names.contains(&"brand_new".to_string()), "{names:?}");
            assert!(
                !names.iter().any(|n| n.starts_with("extra")),
                "the content of an old extra field is free: {names:?}"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert_eq!(
        repo.get("account").await.unwrap().rev,
        3,
        "the refusal wrote nothing"
    );
}

/// An invalid value of a known field in an old document is still rejected.
#[tokio::test]
async fn legacy_document_still_checks_known_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (db, repo) = open(dir.path(), 1);
    seed_legacy(&db).await;
    assert!(matches!(
        repo.patch("account", json!({"language": "xx"}), 1).await,
        Err(SettingsError::Invalid(_))
    ));
}
