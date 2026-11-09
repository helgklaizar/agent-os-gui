//! Retention (A7-08, docs/reference/db.md section 7): cleanup by age in batches. Each batch is one writer transaction of at most `BATCH`
//! rows; the loop runs while the batch is full. The action log is not deleted: old rows get `args_json` nulled.

use std::path::PathBuf;
use std::sync::Arc;

use crate::{Db, DbError};

const DAY: i64 = 86_400;

/// How many rows one step (one transaction) touches.
pub const BATCH: i64 = 1000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub events: u64,
    pub args_cleared: u64,
    pub task_runs: u64,
    pub gw_isolated: u64,
    pub findings: u64,
    pub quota_samples: u64,
    /// How many migration snapshots were moved to `*.bak-YYYYMMDD` (files are not deleted).
    pub snapshots_moved: u64,
    /// How many batch transactions were executed.
    pub batches: u64,
    /// Result of `PRAGMA integrity_check` after cleanup (S10-DB-05).
    pub integrity: String,
    /// Rows in `PRAGMA foreign_key_check` after cleanup (S10-DB-05).
    pub fk_violations: u64,
    /// Snapshot taken before `DELETE` (S10-DB-04).
    pub snapshot: Option<PathBuf>,
}

impl RetentionReport {
    /// Integrity after cleanup is fine.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.integrity == "ok" && self.fk_violations == 0
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.events
            + self.args_cleared
            + self.task_runs
            + self.gw_isolated
            + self.findings
            + self.quota_samples
            + self.snapshots_moved
    }
}

/// How many `pre-retention-*.db` snapshots are kept; older ones are moved to `*.bak-YYYYMMDD` (assumption S10-DB-04).
pub const SNAPSHOTS_KEEP: usize = 3;

pub struct Retention {
    db: Arc<Db>,
    snapshots: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
}

impl Retention {
    #[must_use]
    pub fn new(db: Arc<Db>) -> Retention {
        Retention {
            db,
            snapshots: None,
            snapshot_dir: None,
        }
    }

    /// Before cleanup, `pre-retention-<time>.db` is taken into `dir` (S10-DB-04).
    #[must_use]
    pub fn with_snapshots(mut self, dir: impl Into<PathBuf>) -> Retention {
        self.snapshots = Some(dir.into());
        self
    }

    /// `VACUUM INTO` by a separate reading connection (the writer is not blocked); directory `0700`, file `0600`.
    async fn snapshot(&self, dir: PathBuf, now: i64) -> Result<PathBuf, DbError> {
        let path = self.db.path().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            crate::snapshot::take_with(
                &conn,
                &dir,
                "pre-retention-",
                now,
                SNAPSHOTS_KEEP,
                &crate::snapshot::today_stamp(),
            )
        })
        .await
        .map_err(|e| DbError::Io(std::io::Error::other(e.to_string())))?
    }

    /// Migration snapshot directory (`migration-backups/`): "two latest" retention is applied here too.
    #[must_use]
    pub fn with_snapshot_dir(mut self, dir: PathBuf) -> Retention {
        self.snapshot_dir = Some(dir);
        self
    }

    async fn step(
        &self,
        batches: &mut u64,
        sql: &'static str,
        args: Vec<i64>,
    ) -> Result<u64, DbError> {
        let mut total = 0u64;
        loop {
            let a = args.clone();
            let n = self
                .db
                .write(move |tx| {
                    let params: Vec<&dyn rusqlite::ToSql> =
                        a.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                    tx.execute(sql, params.as_slice())
                })
                .await?;
            *batches += 1;
            total += n as u64;
            if (n as i64) < BATCH {
                return Ok(total);
            }
        }
    }

    /// Applies the age limits as of `now` (unix seconds). A repeated run without new data changes nothing.
    pub async fn run(&self, now: i64) -> Result<RetentionReport, DbError> {
        let snapshot = match self.snapshots.clone() {
            Some(dir) => Some(self.snapshot(dir, now).await?),
            None => None,
        };
        let mut b = 0u64;
        let events = self
            .step(
                &mut b,
                "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE unread = 0 AND level <> 'error' AND ts < ?1 LIMIT ?2)",
                vec![now - 90 * DAY, BATCH],
            )
            .await?;
        let args_cleared = self
            .step(
                &mut b,
                "UPDATE actions_log SET args_json = NULL WHERE id IN (SELECT id FROM actions_log WHERE args_json IS NOT NULL AND ts < ?1 LIMIT ?2)",
                vec![now - 90 * DAY, BATCH],
            )
            .await?;
        let task_runs = self
            .step(
                &mut b,
                "DELETE FROM task_runs WHERE id IN (
                   SELECT id FROM (SELECT id, row_number() OVER (PARTITION BY task_id ORDER BY id DESC) AS rn FROM task_runs)
                   WHERE rn > 200 LIMIT ?1)",
                vec![BATCH],
            )
            .await?;
        let gw_isolated = self
            .step(
                &mut b,
                "DELETE FROM gw_messages WHERE id IN (SELECT id FROM gw_messages WHERE isolated = 1 AND ts < ?1 LIMIT ?2)",
                vec![now - 30 * DAY, BATCH],
            )
            .await?;
        let findings = self
            .step(
                &mut b,
                "DELETE FROM observer_findings WHERE id IN (SELECT id FROM observer_findings WHERE ts < ?1 LIMIT ?2)",
                vec![now - 365 * DAY, BATCH],
            )
            .await?;
        // Quota samples: older than 30 days: delete; older than a day: keep one point (the first) per 15 minutes.
        let old = self
            .step(
                &mut b,
                "DELETE FROM quota_samples WHERE (account_id, ts) IN (SELECT account_id, ts FROM quota_samples WHERE ts < ?1 LIMIT ?2)",
                vec![now - 30 * DAY, BATCH],
            )
            .await?;
        let thin = self
            .step(
                &mut b,
                "DELETE FROM quota_samples WHERE (account_id, ts) IN (
                   SELECT q.account_id, q.ts FROM quota_samples q WHERE q.ts < ?1 AND EXISTS (
                     SELECT 1 FROM quota_samples p WHERE p.account_id = q.account_id AND p.ts < q.ts
                       AND p.ts / 900 = q.ts / 900) LIMIT ?2)",
                vec![now - DAY, BATCH],
            )
            .await?;
        let check = self.db.read(crate::check_conn).await?;
        let snapshots_moved = match self.snapshot_dir.clone() {
            Some(dir) => {
                let stamp = crate::snapshot::stamp_from_unix(now);
                tokio::task::spawn_blocking(move || crate::snapshot::rotate(&dir, &stamp))
                    .await
                    .map_err(|e| DbError::Io(std::io::Error::other(e)))??
            }
            None => 0,
        };
        let r = RetentionReport {
            events,
            args_cleared,
            task_runs,
            gw_isolated,
            findings,
            quota_samples: old + thin,
            snapshots_moved,
            batches: b,
            integrity: check.integrity,
            fk_violations: check.fk_violations,
            snapshot,
        };
        tracing::info!(
            scope = "retention",
            event = "done",
            events = r.events,
            args_cleared = r.args_cleared,
            task_runs = r.task_runs,
            gw_isolated = r.gw_isolated,
            findings = r.findings,
            quota_samples = r.quota_samples,
            integrity_ok = r.ok(),
            snapshots_moved = r.snapshots_moved,
            "retention done"
        );
        Ok(r)
    }
}
