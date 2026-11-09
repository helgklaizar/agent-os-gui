//! A1-03: the single writer, queue, readers, permissions, instance lock.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use agentd_db::{Db, DbConfig, DbError};

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn open_in(dir: &Path) -> Db {
    Db::open(DbConfig::new(dir.join("agentos/sessions.db"))).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_serializes_concurrent_writes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(open_in(dir.path()));
    db.write(|tx| {
        tx.execute_batch("CREATE TABLE t_log(id INTEGER PRIMARY KEY, seen INTEGER NOT NULL);")
    })
    .await
    .unwrap();
    let mut tasks = Vec::new();
    for _ in 0..100 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            // read-modify-write inside one write: without serialization, updates would be lost
            db.write(|tx| {
                let n: i64 = tx.query_row("SELECT count(*) FROM t_log", [], |r| r.get(0))?;
                tx.execute("INSERT INTO t_log(seen) VALUES (?1)", [n])?;
                Ok(n)
            })
            .await
        }));
    }
    let mut seen = Vec::new();
    for t in tasks {
        seen.push(t.await.unwrap().expect("write without SQLITE_BUSY"));
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..100).collect::<Vec<i64>>(),
        "each write saw all the previous ones"
    );
    let total: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM t_log", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(total, 100);
    let ok: String = db
        .read(|c| c.query_row("PRAGMA integrity_check", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(ok, "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_queue_backpressure() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = DbConfig::new(dir.path().join("agentos/sessions.db"));
    cfg.queue_len = 2;
    let db = Arc::new(Db::open(cfg).unwrap());
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let blocker = {
        let db = db.clone();
        tokio::spawn(async move {
            db.write(move |_| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
        })
    };
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the writer is busy with a blocking write");
    let mut queued = Vec::new();
    for _ in 0..2 {
        let db = db.clone();
        queued.push(tokio::spawn(async move { db.write(|_| Ok(())).await }));
    }
    for _ in 0..200 {
        if db.queue_len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(db.queue_len(), 2, "queue is full");
    let overflow = db.write(|_| Ok(())).await;
    assert!(matches!(overflow, Err(DbError::QueueFull)), "{overflow:?}");
    release_tx.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    for q in queued {
        q.await.unwrap().expect("the queued write was executed");
    }
    db.write(|_| Ok(()))
        .await
        .expect("after draining, the core accepts writes");
    assert_eq!(db.queue_len(), 0);
}

#[tokio::test]
async fn db_files_permissions_0600() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("agentos");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    let db = open_in(dir.path());
    db.write(|tx| tx.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);"))
        .await
        .unwrap();
    let base = home.join("sessions.db");
    assert_eq!(mode(&home), 0o700, "directory");
    for suffix in ["", "-wal", "-shm", ".lock"] {
        let mut name = base.clone().into_os_string();
        name.push(suffix);
        let p = std::path::PathBuf::from(name);
        assert!(p.exists(), "{} must exist", p.display());
        assert_eq!(mode(&p), 0o600, "{}", p.display());
    }
}

#[tokio::test]
async fn db_dir_created_0700_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let _db = open_in(dir.path());
    assert_eq!(mode(&dir.path().join("agentos")), 0o700);
}

#[tokio::test]
async fn second_instance_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let first = open_in(dir.path());
    let second = Db::open(DbConfig::new(dir.path().join("agentos/sessions.db")));
    assert!(
        matches!(second, Err(DbError::AlreadyRunning)),
        "{:?}",
        second.err()
    );
    first
        .write(|_| Ok(()))
        .await
        .expect("the first instance is alive");
    drop(first);
    assert!(
        Db::open(DbConfig::new(dir.path().join("agentos/sessions.db"))).is_ok(),
        "after the first one stops, a start is possible"
    );
}

#[tokio::test]
async fn write_error_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_in(dir.path());
    db.write(|tx| tx.execute_batch("CREATE TABLE t(x INTEGER);"))
        .await
        .unwrap();
    let err = db
        .write(|tx| {
            tx.execute("INSERT INTO t VALUES (1)", [])?;
            tx.execute("INSERT INTO no_such_table VALUES (1)", [])?;
            Ok(())
        })
        .await;
    assert!(matches!(err, Err(DbError::Sqlite(_))), "{err:?}");
    let n: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 0, "the transaction rolled back entirely");
}

#[tokio::test]
async fn writer_survives_panicking_closure() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_in(dir.path());
    db.write(|tx| tx.execute_batch("CREATE TABLE t(x INTEGER);"))
        .await
        .unwrap();
    let res: Result<(), DbError> = db
        .write(|tx| {
            tx.execute("INSERT INTO t VALUES (1)", [])?;
            panic!("failure in the handler");
        })
        .await;
    assert!(matches!(res, Err(DbError::WriterPanicked)), "{res:?}");
    db.write(|tx| tx.execute("INSERT INTO t VALUES (2)", []))
        .await
        .expect("the writer is alive");
    let rows: Vec<i64> = db
        .read(|c| {
            c.prepare("SELECT x FROM t")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()
        })
        .await
        .unwrap();
    assert_eq!(rows, vec![2]);
}

#[tokio::test]
async fn reader_is_read_only_and_sees_committed_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_in(dir.path());
    db.write(|tx| tx.execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES (7);"))
        .await
        .unwrap();
    let v: i64 = db
        .read(|c| c.query_row("SELECT x FROM t", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(v, 7);
    let attempt = db.read(|c| c.execute("INSERT INTO t VALUES (8)", [])).await;
    assert!(attempt.is_err(), "a reader does not write");
}

#[tokio::test]
async fn connection_pragmas_match_db_md() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_in(dir.path());
    let (jm, sync, fk, busy): (String, i64, i64, i64) = db
        .write(|tx| {
            Ok((
                tx.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
                tx.query_row("PRAGMA synchronous", [], |r| r.get(0))?,
                tx.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
                tx.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?,
            ))
        })
        .await
        .unwrap();
    assert_eq!((jm.as_str(), sync, fk, busy), ("wal", 1, 1, 5000));
}
