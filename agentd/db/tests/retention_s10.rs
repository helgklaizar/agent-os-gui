//! S10-DB-04 / S10-DB-05 / S10-11: retention takes a snapshot before `DELETE` and checks integrity after cleanup;
//! `messages` and `actions_log` rows live forever.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use agentd_db::{Connection, Db, DbConfig, Retention, params};

const DAY: i64 = 86_400;
const NOW: i64 = 2_000_000_000;

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

struct Rig {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    db: Arc<Db>,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("agentos");
    let db = Arc::new(Db::open(DbConfig::new(root.join("sessions.db"))).unwrap());
    Rig {
        _dir: dir,
        root,
        db,
    }
}

async fn old_events(r: &Rig, n: i64) {
    r.db.write(move |tx| {
        for i in 0..n {
            tx.execute(
                "INSERT INTO events(kind, sub, level, text, unread, ts) VALUES ('msg','sys','info','old',0,?1)",
                [NOW - 200 * DAY + i],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

fn count_in(path: &Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |x| x.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_snapshots_before_delete_and_snapshot_keeps_old_rows() {
    let r = rig().await;
    old_events(&r, 7).await;
    let dir = r.root.join("retention-backups");
    let ret = Retention::new(r.db.clone()).with_snapshots(&dir);
    let rep = ret.run(NOW).await.unwrap();
    assert_eq!(rep.events, 7);
    let snap = rep.snapshot.expect("snapshot taken before cleanup");
    assert!(snap.starts_with(&dir), "{snap:?}");
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&snap), 0o600);
    assert_eq!(
        count_in(&snap, "SELECT count(*) FROM events"),
        7,
        "the snapshot holds rows deleted from the live DB"
    );
    assert_eq!(
        count_in(&r.root.join("sessions.db"), "SELECT count(*) FROM events"),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_snapshots_keep_three_latest_and_move_older_to_bak() {
    let r = rig().await;
    let dir = r.root.join("retention-backups");
    let ret = Retention::new(r.db.clone()).with_snapshots(&dir);
    for i in 0..5 {
        ret.run(NOW + i * 60).await.unwrap();
    }
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let live = names.iter().filter(|n| n.ends_with(".db")).count();
    let moved = names.iter().filter(|n| n.contains(".db.bak-")).count();
    assert_eq!(live, 3, "{names:?}");
    assert_eq!(moved, 2, "extra snapshots moved, not deleted: {names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_without_snapshot_dir_takes_none() {
    let r = rig().await;
    old_events(&r, 1).await;
    let rep = Retention::new(r.db.clone()).run(NOW).await.unwrap();
    assert!(rep.snapshot.is_none());
    assert!(!r.root.join("retention-backups").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_reports_clean_integrity_after_run() {
    let r = rig().await;
    old_events(&r, 3).await;
    let rep = Retention::new(r.db.clone()).run(NOW).await.unwrap();
    assert_eq!(rep.integrity, "ok");
    assert_eq!(rep.fk_violations, 0);
    assert!(rep.ok(), "{rep:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_reports_broken_foreign_keys() {
    let r = rig().await;
    {
        // Orphan: a separate connection where foreign key checking is off by default.
        let c = Connection::open(r.root.join("sessions.db")).unwrap();
        c.execute(
            "INSERT INTO messages(session_id, role, body_md, ts) VALUES (999, 'user', 'orphan', 1)",
            [],
        )
        .unwrap();
    }
    let rep = Retention::new(r.db.clone()).run(NOW).await.unwrap();
    assert_eq!(rep.fk_violations, 1, "{rep:?}");
    assert!(!rep.ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_keeps_messages_and_action_rows_forever() {
    let r = rig().await;
    r.db.write(|tx| {
        tx.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
            [],
        )?;
        for ts in [1, NOW - 5 * 365 * DAY] {
            tx.execute(
                "INSERT INTO messages(session_id, role, body_md, ts) VALUES (1, 'user', 'ancient', ?1)",
                [ts],
            )?;
            tx.execute(
                "INSERT INTO actions_log(ts, actor, action, args_json, decision) VALUES (?1, 'agent', 'read', NULL, 'allow')",
                params![ts],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let rep = Retention::new(r.db.clone()).run(NOW).await.unwrap();
    assert_eq!(rep.total(), 0, "{rep:?}");
    let p = r.root.join("sessions.db");
    assert_eq!(count_in(&p, "SELECT count(*) FROM messages"), 2);
    assert_eq!(count_in(&p, "SELECT count(*) FROM actions_log"), 2);
}
