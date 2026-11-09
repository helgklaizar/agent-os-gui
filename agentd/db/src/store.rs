//! Store with a single writer (docs/reference/db.md section 2, docs/adr/0002-db-single-writer.md).
//!
//! Write: the closure goes into a bounded queue (`tokio::sync::mpsc`) and one thread runs it on the single
//! write connection inside a transaction. A full queue gives `DbError::QueueFull` (the API answers `503 core_down`).
//! Read: a read-only connection pool, queries run through `spawn_blocking`.

use std::fs::File;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use tokio::sync::{mpsc, oneshot};

use crate::{DbError, MIGRATIONS, MigrateOptions, migrate, storage};

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

pub struct DbConfig {
    pub path: PathBuf,
    /// Write queue length (assumption A1-03: 1024).
    pub queue_len: usize,
    /// How many read connections to keep in the pool.
    pub read_pool: usize,
    pub migrate: MigrateOptions,
}

impl DbConfig {
    /// Defaults; migration snapshots go to `migration-backups/` next to the DB (docs/reference/db.md section 6).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> DbConfig {
        let path = path.into();
        let migrate = MigrateOptions {
            backup_dir: Some(storage::parent_of(&path).join("migration-backups")),
            ..MigrateOptions::default()
        };
        DbConfig {
            path,
            queue_len: 1024,
            read_pool: 4,
            migrate,
        }
    }
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

struct Readers {
    path: PathBuf,
    max: usize,
    pool: Mutex<Vec<Connection>>,
}

impl Readers {
    fn take(&self) -> Result<Connection, DbError> {
        if let Some(c) = self.pool.lock().unwrap_or_else(|e| e.into_inner()).pop() {
            return Ok(c);
        }
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(conn)
    }

    fn give_back(&self, conn: Connection) {
        let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        if pool.len() < self.max {
            pool.push(conn);
        }
    }
}

/// DB with a single writer.
pub struct Db {
    path: PathBuf,
    jobs: Option<mpsc::Sender<Job>>,
    writer: Option<JoinHandle<()>>,
    readers: Arc<Readers>,
    // Instance lock: the field is declared last, so it is released after the writer stops in `Drop`.
    _lock: File,
}

fn expect_pragma(name: &'static str, got: String, want: &str) -> Result<(), DbError> {
    if got.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err(DbError::Pragma { name, got })
    }
}

fn open_writer(path: &Path) -> Result<Connection, DbError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    expect_pragma("journal_mode", mode, "wal")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    expect_pragma("foreign_keys", fk.to_string(), "1")?;
    Ok(conn)
}

impl Db {
    /// Opens the DB: directory and files with permissions, instance lock, PRAGMAs, migrations, writer thread.
    pub fn open(cfg: DbConfig) -> Result<Db, DbError> {
        let lock = storage::prepare(&cfg.path)?;
        let mut conn = open_writer(&cfg.path)?;
        migrate(&mut conn, MIGRATIONS, &cfg.migrate)?;
        storage::finish(&cfg.path)?;
        let (tx, mut rx) = mpsc::channel::<Job>(cfg.queue_len.max(1));
        let writer = std::thread::Builder::new()
            .name("agentd-db-writer".into())
            .spawn(move || {
                while let Some(job) = rx.blocking_recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Db {
            readers: Arc::new(Readers {
                path: cfg.path.clone(),
                max: cfg.read_pool.max(1),
                pool: Mutex::new(Vec::new()),
            }),
            path: cfg.path,
            jobs: Some(tx),
            writer: Some(writer),
            _lock: lock,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many writes wait in the queue (not counting the one running).
    #[must_use]
    pub fn queue_len(&self) -> usize {
        self.jobs
            .as_ref()
            .map_or(0, |j| j.max_capacity() - j.capacity())
    }

    /// The single write point: the closure runs in a transaction on the single write connection.
    /// `Err` from the closure or a panic rolls everything back.
    pub async fn write<T, F>(&self, f: F) -> Result<T, DbError>
    where
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (done_tx, done_rx) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<T, DbError> {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let value = f(&tx)?;
                tx.commit()?;
                Ok(value)
            }))
            .unwrap_or(Err(DbError::WriterPanicked));
            // The receiver is closed if the caller canceled the wait: the write is already done or rolled back, nobody to notify.
            drop(done_tx.send(outcome));
        });
        let jobs = self.jobs.as_ref().ok_or(DbError::WriterGone)?;
        jobs.try_send(job).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => DbError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => DbError::WriterGone,
        })?;
        done_rx.await.map_err(|_| DbError::WriterGone)?
    }

    /// Read on a separate read-only connection (`query_only`).
    pub async fn read<T, F>(&self, f: F) -> Result<T, DbError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let readers = self.readers.clone();
        tokio::task::spawn_blocking(move || {
            let conn = readers.take()?;
            let result = f(&conn);
            readers.give_back(conn);
            Ok(result?)
        })
        .await
        .map_err(|_| DbError::ReaderPanicked)?
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(w) = self.writer.take() {
            // The writer thread exits when the queue is closed; a panic inside closures is caught above.
            drop(w.join());
        }
    }
}
