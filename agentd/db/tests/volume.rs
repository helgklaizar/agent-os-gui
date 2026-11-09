//! A5-13 (Q5): measuring `sessions.db` volumes on a synthetic load for `AGENTD_VOLUME_DAYS` days (90 by default).
//! The profile is an assumption of this test (see the constants); the numbers are printed as `VOLUME …` lines for the report (`--nocapture`).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agentd_db::{Db, DbConfig, params};

const DAY: i64 = 86_400;
const EVENTS_PER_DAY: i64 = 150;
const GW_PER_DAY: i64 = 80;
const RUNS_PER_DAY: i64 = 40;
const QUOTA_ACCOUNTS: i64 = 3;
const QUOTA_SAMPLES_PER_DAY: i64 = 288;
const MESSAGES_PER_DAY: i64 = 200;
const ACTIONS_PER_DAY: i64 = 100;

type Filler = dyn Fn(&agentd_db::Transaction<'_>, i64) -> agentd_db::SqlResult<()> + Send;

fn size(db: &Path) -> u64 {
    std::fs::metadata(db).map_or(0, |m| m.len())
}

/// Flush the WAL into the main file so the file size reflects the data (a separate connection outside the writer's transaction).
fn checkpoint(path: &Path) {
    let c = agentd_db::Connection::open(path).unwrap();
    c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn volume_90_days_synthetic() {
    let days: i64 = std::env::var("AGENTD_VOLUME_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agentos/sessions.db");
    let db = Arc::new(Db::open(DbConfig::new(&path)).unwrap());
    let t0 = 1_700_000_000_i64;
    db.write(move |tx| {
        for i in 1..=5 {
            tx.execute(
                "INSERT INTO sessions(name, created_at, updated_at) VALUES (?1, ?2, ?2)",
                params![format!("s{i}"), t0],
            )?;
        }
        for i in 1..=QUOTA_ACCOUNTS {
            tx.execute(
                "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'tg', 'a', ?2)",
                params![format!("a{i}"), t0],
            )?;
        }
        tx.execute(
            "INSERT INTO tasks(kind, title, spec, created_at) VALUES ('sched','t','s',?1)",
            [t0],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    checkpoint(&path);
    let base = size(&path);
    println!("VOLUME days={days} base_bytes={base}");

    let mut rows: Vec<(&str, i64, u64)> = Vec::new();
    let mut last = base;
    let text: Arc<str> = "é".repeat(150).into(); // 300 bytes of 2-byte characters
    let out: Arc<str> = "o".repeat(500).into();
    let args = r#"{"target":"~/notes/a.md","note":"synthetic"}"#;
    let (t, o) = (text.clone(), out.clone());
    let plan: Vec<(&str, i64, Box<Filler>)> = vec![
        (
            "events",
            days * EVENTS_PER_DAY,
            Box::new(move |tx, i| {
                tx.execute(
                "INSERT INTO events(kind, sub, level, text, session_id, dedup_key, unread, ts) VALUES ('msg','tg','info',?1,?2,?3,0,?4)",
                params![&*t, 1 + i % 5, format!("k:{i}"), t0 + i * DAY / EVENTS_PER_DAY],
            ).map(|_| ())
            }),
        ),
        ("gw_messages", days * GW_PER_DAY, {
            let t = text.clone();
            Box::new(move |tx, i| {
                tx.execute(
                    "INSERT INTO gw_messages(account_id, ext_id, direction, body, isolated, read, ts) VALUES (?1, ?2, 'in', ?3, ?4, 1, ?5)",
                    params![1 + i % QUOTA_ACCOUNTS, format!("e{i}"), &*t, i64::from(i % 10 == 0), t0 + i * DAY / GW_PER_DAY],
                ).map(|_| ())
            })
        }),
        (
            "task_runs",
            days * RUNS_PER_DAY,
            Box::new(move |tx, i| {
                tx.execute(
                "INSERT INTO task_runs(task_id, started_at, finished_at, status, output) VALUES (1, ?1, ?1, 'ok', ?2)",
                params![t0 + i * DAY / RUNS_PER_DAY, &*o],
            ).map(|_| ())
            }),
        ),
        (
            "quota_samples",
            days * QUOTA_ACCOUNTS * QUOTA_SAMPLES_PER_DAY,
            Box::new(move |tx, i| {
                tx.execute(
                "INSERT INTO quota_samples(account_id, ts, used, usage_tokens, cost) VALUES (?1, ?2, 0.5, 1000, 0.01)",
                params![1 + i % QUOTA_ACCOUNTS, t0 + i],
            ).map(|_| ())
            }),
        ),
        ("messages", days * MESSAGES_PER_DAY, {
            let t = text.clone();
            Box::new(move |tx, i| {
                tx.execute(
                    "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES (?1, 'agent', ?2, 'done', ?3)",
                    params![1 + i % 5, &*t, t0 + i * DAY / MESSAGES_PER_DAY],
                ).map(|_| ())
            })
        }),
        (
            "actions_log",
            days * ACTIONS_PER_DAY,
            Box::new(move |tx, i| {
                tx.execute(
                "INSERT INTO actions_log(ts, actor, session_id, action, target, args_json, decision, result, reversible) VALUES (?1,'agent',?2,'read','~/a.md',?3,'allow','ok',0)",
                params![t0 + i * DAY / ACTIONS_PER_DAY, 1 + i % 5, args],
            ).map(|_| ())
            }),
        ),
    ];
    for (name, n, f) in plan {
        db.write(move |tx| {
            for i in 0..n {
                f(tx, i)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        checkpoint(&path);
        let now = size(&path);
        rows.push((name, n, now.saturating_sub(last)));
        last = now;
    }
    for (name, n, bytes) in &rows {
        println!("VOLUME table={name} rows={n} bytes={bytes}");
    }
    let total = size(&path);
    println!(
        "VOLUME total_bytes={total} per_year_bytes={}",
        total * 365 / days as u64
    );

    // A snapshot by the real code (`VACUUM INTO` on a separate connection, as before migrations).
    let conn = agentd_db::Connection::open(&path).unwrap();
    let t = Instant::now();
    let target = agentd_db::snapshot::take(&conn, dir.path(), 6, "volume").unwrap();
    let vacuum_ms = t.elapsed().as_millis();
    println!(
        "VOLUME vacuum_into_ms={vacuum_ms} backup_bytes={}",
        size(&target)
    );
    assert!(target.exists(), "VACUUM INTO did not create the file");
    let ok: String = db
        .read(|c| c.query_row("PRAGMA integrity_check", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(ok, "ok");
    let counted: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM events", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(counted, days * EVENTS_PER_DAY);
}
