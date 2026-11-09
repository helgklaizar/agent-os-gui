//! A7-08: migration snapshot retention (db.md section 6, Q5): the two latest `pre-migration-*.db` are kept, the rest are moved to
//! `*.bak-YYYYMMDD`; files are not deleted, contents stay intact, foreign files are untouched, `.bak` is not overwritten.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentd_db::{Db, DbConfig, Retention, snapshot};

const NOW: i64 = 2_000_000_000;

struct Rig {
    _dir: tempfile::TempDir,
    snaps: PathBuf,
    db: Arc<Db>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("agentos/sessions.db"))).unwrap());
    let snaps = dir.path().join("migration-backups");
    std::fs::create_dir_all(&snaps).unwrap();
    Rig {
        _dir: dir,
        snaps,
        db,
    }
}

fn put(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

/// Spec criterion A7-08 `retention_snapshots_moved_not_deleted`: three snapshots, the two latest stay, the old one is moved.
#[tokio::test(flavor = "multi_thread")]
async fn retention_snapshots_moved_not_deleted() {
    let r = rig();
    put(&r.snaps, "pre-migration-2.db", "snapshot-two");
    put(&r.snaps, "pre-migration-3.db", "snapshot-three");
    put(&r.snaps, "pre-migration-4.db", "snapshot-four");
    put(&r.snaps, "notes.txt", "not a snapshot");
    let ret = Retention::new(r.db.clone()).with_snapshot_dir(r.snaps.clone());
    let stamp = snapshot::stamp_from_unix(NOW);

    let rep = ret.run(NOW).await.unwrap();
    assert_eq!(rep.snapshots_moved, 1, "{rep:?}");
    let moved = format!("pre-migration-2.db.bak-{stamp}");
    assert_eq!(
        names(&r.snaps),
        vec![
            "notes.txt".to_string(),
            moved.clone(),
            "pre-migration-3.db".to_string(),
            "pre-migration-4.db".to_string()
        ],
        "the two latest in place, the old one renamed, no file lost"
    );
    assert_eq!(
        std::fs::read_to_string(r.snaps.join(&moved)).unwrap(),
        "snapshot-two",
        "the moved snapshot's contents are intact"
    );

    let again = ret.run(NOW).await.unwrap();
    assert_eq!(again.snapshots_moved, 0);
    assert_eq!(names(&r.snaps).len(), 4, "a repeated run changes nothing");
}

/// Two snapshots: nothing is moved; a taken `.bak-<date>` name is not overwritten (it gets a suffix).
#[tokio::test(flavor = "multi_thread")]
async fn retention_snapshots_keep_two_and_never_overwrite_bak() {
    let r = rig();
    put(&r.snaps, "pre-migration-5.db", "five");
    put(&r.snaps, "pre-migration-6.db", "six");
    let ret = Retention::new(r.db.clone()).with_snapshot_dir(r.snaps.clone());
    assert_eq!(ret.run(NOW).await.unwrap().snapshots_moved, 0);
    assert_eq!(names(&r.snaps).len(), 2);

    let stamp = snapshot::stamp_from_unix(NOW);
    put(&r.snaps, "pre-migration-7.db", "seven");
    put(&r.snaps, "pre-migration-4.db", "four");
    let taken = format!("pre-migration-4.db.bak-{stamp}");
    put(&r.snaps, &taken, "older-bak");
    let rep = ret.run(NOW).await.unwrap();
    assert_eq!(rep.snapshots_moved, 2, "{rep:?}");
    assert_eq!(
        std::fs::read_to_string(r.snaps.join(&taken)).unwrap(),
        "older-bak",
        "the previous .bak is not overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(r.snaps.join(format!("{taken}-2"))).unwrap(),
        "four"
    );
    assert_eq!(names(&r.snaps).len(), 5, "all five files in place");
}

/// No directory set: snapshot retention looks for nothing and does nothing.
#[tokio::test(flavor = "multi_thread")]
async fn retention_without_snapshot_dir_touches_no_files() {
    let r = rig();
    put(&r.snaps, "pre-migration-1.db", "a");
    put(&r.snaps, "pre-migration-2.db", "b");
    put(&r.snaps, "pre-migration-3.db", "c");
    let rep = Retention::new(r.db.clone()).run(NOW).await.unwrap();
    assert_eq!(rep.snapshots_moved, 0);
    assert_eq!(names(&r.snaps).len(), 3);
    assert!(r.snaps.join("pre-migration-1.db").exists());
}
