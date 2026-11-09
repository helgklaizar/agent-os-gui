//! A7-08: retention per db.md section 7 on controlled time: read non-`error` events older than 90 days, the log's `args_json`
//! older than 90 days is nulled (the row stays), `task_runs` exactly 200 per task, isolated gateway messages 30 days,
//! findings 1 year, quota samples 30 days with thinning; batches are bounded; a repeated run changes nothing.

use std::sync::Arc;

use agentd_db::{Db, DbConfig, Retention, params};

const DAY: i64 = 86_400;
const NOW: i64 = 2_000_000_000;

struct Rig {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    ret: Retention,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("agentos/sessions.db"))).unwrap());
    Rig {
        ret: Retention::new(db.clone()),
        db,
        _dir: dir,
    }
}

async fn count(r: &Rig, sql: &'static str) -> i64 {
    r.db.read(move |c| c.query_row(sql, [], |x| x.get(0)))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_events_keep_unread_error_and_fresh() {
    let r = rig().await;
    r.db.write(|tx| {
        let old = NOW - 91 * DAY;
        for (level, unread, ts) in [
            ("info", 0, old),
            ("ok", 0, old),
            ("error", 0, old),
            ("info", 1, old),
            ("info", 0, NOW - 89 * DAY),
        ] {
            tx.execute(
                "INSERT INTO events(kind, sub, level, text, unread, ts) VALUES ('msg','sys',?1,'t',?2,?3)",
                params![level, unread, ts],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let rep = r.ret.run(NOW).await.unwrap();
    assert_eq!(
        rep.events, 2,
        "only read non-error events older than 90 days were deleted"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM events").await, 3);
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM events WHERE level = 'error' OR unread = 1"
        )
        .await,
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_actions_log_keeps_rows_and_nulls_old_args() {
    let r = rig().await;
    r.db.write(|tx| {
        for (ts, args) in [(NOW - 100 * DAY, Some("{\"a\":1}")), (NOW - 10 * DAY, Some("{\"b\":2}"))] {
            tx.execute(
                "INSERT INTO actions_log(ts, actor, action, args_json, decision) VALUES (?1, 'agent', 'read', ?2, 'allow')",
                params![ts, args],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let rep = r.ret.run(NOW).await.unwrap();
    assert_eq!(rep.args_cleared, 1);
    assert_eq!(
        count(&r, "SELECT count(*) FROM actions_log").await,
        2,
        "the log stays"
    );
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM actions_log WHERE args_json IS NULL"
        )
        .await,
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_task_runs_exactly_200_per_task() {
    let r = rig().await;
    r.db.write(|tx| {
        tx.execute("INSERT INTO tasks(kind, title, spec, created_at) VALUES ('deleg','a','s',1), ('deleg','b','s',1)", [])?;
        for task in [1, 2] {
            let n = if task == 1 { 250 } else { 50 };
            for i in 0..n {
                tx.execute(
                    "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1, ?2, 'ok')",
                    params![task, i],
                )?;
            }
        }
        Ok(())
    })
    .await
    .unwrap();
    let rep = r.ret.run(NOW).await.unwrap();
    assert_eq!(rep.task_runs, 50);
    assert_eq!(
        count(&r, "SELECT count(*) FROM task_runs WHERE task_id = 1").await,
        200
    );
    assert_eq!(
        count(&r, "SELECT count(*) FROM task_runs WHERE task_id = 2").await,
        50
    );
    assert_eq!(
        count(
            &r,
            "SELECT min(started_at) FROM task_runs WHERE task_id = 1"
        )
        .await,
        50,
        "the newest remain"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_gateway_findings_and_quota_samples() {
    let r = rig().await;
    r.db.write(|tx| {
        tx.execute("INSERT INTO accounts(type, label, slug, created_at) VALUES ('tg','a','a',1)", [])?;
        for (ext, iso, ts) in [("1", 1, NOW - 31 * DAY), ("2", 1, NOW - 29 * DAY), ("3", 0, NOW - 400 * DAY)] {
            tx.execute(
                "INSERT INTO gw_messages(account_id, ext_id, direction, body, isolated, ts) VALUES (1, ?1, 'in', 'x', ?2, ?3)",
                params![ext, iso, ts],
            )?;
        }
        for ts in [NOW - 366 * DAY, NOW - 100 * DAY] {
            tx.execute(
                "INSERT INTO observer_findings(ts, kind, severity, verdict) VALUES (?1, 'injection', 'high', 'blocked')",
                [ts],
            )?;
        }
        // Samples: 40 days ago (delete), 3 days ago with 4 points per 15 minutes (keep one), fresh (keep).
        for i in 0..4 {
            tx.execute("INSERT INTO quota_samples(account_id, ts) VALUES (1, ?1)", [NOW - 3 * DAY + i * 60])?;
        }
        tx.execute("INSERT INTO quota_samples(account_id, ts) VALUES (1, ?1)", [NOW - 40 * DAY])?;
        tx.execute("INSERT INTO quota_samples(account_id, ts) VALUES (1, ?1)", [NOW - 3600])?;
        Ok(())
    })
    .await
    .unwrap();
    let rep = r.ret.run(NOW).await.unwrap();
    assert_eq!(
        rep.gw_isolated, 1,
        "isolated older than 30 days; non-isolated archive stays"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM gw_messages").await, 2);
    assert_eq!(rep.findings, 1);
    assert_eq!(count(&r, "SELECT count(*) FROM observer_findings").await, 1);
    assert_eq!(
        rep.quota_samples, 4,
        "older than 30 days and extra points within 15 minutes"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM quota_samples").await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_gw_isolated_boundary_is_exactly_30_days() {
    // Boundary: exactly 30 days stays, one second older is deleted; 29 days 12 hours (between the 29 and 30 thresholds) stays.
    let r = rig().await;
    r.db.write(|tx| {
        tx.execute("INSERT INTO accounts(type, label, slug, created_at) VALUES ('tg','a','a',1)", [])?;
        for (ext, ts) in [
            ("29d12h", NOW - 29 * DAY - DAY / 2),
            ("30d", NOW - 30 * DAY),
            ("30d+1s", NOW - 30 * DAY - 1),
            ("31d", NOW - 31 * DAY),
        ] {
            tx.execute(
                "INSERT INTO gw_messages(account_id, ext_id, direction, body, isolated, ts) VALUES (1, ?1, 'in', 'x', 1, ?2)",
                params![ext, ts],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let rep = r.ret.run(NOW).await.unwrap();
    assert_eq!(
        rep.gw_isolated, 2,
        "only rows strictly older than 30 days were deleted"
    );
    let left: Vec<String> =
        r.db.read(|c| {
            let mut s = c.prepare("SELECT ext_id FROM gw_messages ORDER BY ts")?;
            s.query_map([], |x| x.get(0))?.collect()
        })
        .await
        .unwrap();
    assert_eq!(left, ["30d", "29d12h"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_is_idempotent_and_batched() {
    let r = rig().await;
    r.db.write(|tx| {
        for i in 0..2500 {
            tx.execute(
                "INSERT INTO events(kind, sub, level, text, unread, ts) VALUES ('msg','sys','info','t',0,?1)",
                [NOW - 200 * DAY + i],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let first = r.ret.run(NOW).await.unwrap();
    assert_eq!(first.events, 2500, "several batches in a row");
    assert!(first.batches >= 3, "batch is bounded: {first:?}");
    let second = r.ret.run(NOW).await.unwrap();
    assert_eq!(second.total(), 0, "a repeated run changes nothing");
}
