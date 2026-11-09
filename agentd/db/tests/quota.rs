//! A3-10 (DB): quota accounting through `QuotaRepo` on a real DB: local model, cold start, remote account.

use std::sync::Arc;

use agentd_core::{LimitWindow, ManualClock, QuotaStore, RateLimit, UsageRecord};
use agentd_db::{Db, DbConfig, QuotaRepo};

const NOW: i64 = 1_800_000_000;

async fn setup() -> (tempfile::TempDir, Arc<Db>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO accounts(id, slug, type, label, created_at) VALUES (1,'ol1','local','Ollama',1),(2,'cl1','anthropic','Claude',1);
             INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'s',1,1);",
        )?;
        for (acc, tin, tout) in [(1, 10, 5), (2, 100, 50)] {
            tx.execute(
                "INSERT INTO messages(session_id, role, body_md, status, meta_json, ts) VALUES (1,'agent','x','done',?1,?2)",
                rusqlite::params![
                    format!(r#"{{"usage":{{"account_id":{acc},"tokens_in":{tin},"tokens_out":{tout}}}}}"#),
                    NOW
                ],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    (dir, db)
}

fn rec(account_id: i64, local: bool, rate: Option<RateLimit>) -> UsageRecord {
    UsageRecord {
        account_id,
        local,
        session_id: 1,
        rate,
        now: NOW,
    }
}

#[tokio::test]
async fn quota_local_has_no_limits() {
    let (_d, db) = setup().await;
    let repo = QuotaRepo::new(db, Arc::new(ManualClock::new(NOW)));
    // Limit headers from a local server are ignored even if present.
    let rl = RateLimit {
        requests: Some(LimitWindow {
            limit: 10,
            remaining: 1,
            reset_at: Some(NOW + 5),
        }),
        tokens: None,
    };
    let q = repo.record(rec(1, true, Some(rl))).await.unwrap();
    assert_eq!(q["kind"], "local");
    assert_eq!(q["window_h"], 0.0);
    for k in ["used", "rate", "eta_at", "reset_at", "limits", "load"] {
        assert!(q[k].is_null(), "{k}: {q}");
    }
    assert_eq!(
        (
            q["usage"]["tokens_in"].as_i64(),
            q["usage"]["tokens_out"].as_i64()
        ),
        (Some(10), Some(5))
    );
}

#[tokio::test]
async fn quota_deleted_account_is_not_recorded() {
    let (_d, db) = setup().await;
    db.write(|tx| {
        tx.execute("UPDATE accounts SET deleted_at = 1 WHERE id = 2", [])
            .map(|_| ())
    })
    .await
    .unwrap();
    let repo = QuotaRepo::new(db.clone(), Arc::new(ManualClock::new(NOW)));
    assert!(repo.record(rec(2, false, None)).await.is_none());
    let n: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM quotas", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn quota_limits_survive_restart_and_missing_headers() {
    let (_d, db) = setup().await;
    let clock = Arc::new(ManualClock::new(NOW));
    let rl = RateLimit {
        requests: Some(LimitWindow {
            limit: 100,
            remaining: 40,
            reset_at: Some(NOW + 20),
        }),
        tokens: Some(LimitWindow {
            limit: 1000,
            remaining: 900,
            reset_at: Some(NOW + 20),
        }),
    };
    let repo = QuotaRepo::new(db.clone(), clock.clone());
    let q = repo.record(rec(2, false, Some(rl))).await.unwrap();
    assert!((q["used"].as_f64().unwrap() - 0.6).abs() < 1e-9);
    // A response without limit headers does not erase the previous ones.
    let q2 = repo.record(rec(2, false, None)).await.unwrap();
    assert_eq!(q2["limits"], q["limits"]);
    assert_eq!(q2["used"], q["used"]);
    // A new instance (after a restart) restores the windows from the columns: the tightest window exactly.
    let fresh = QuotaRepo::new(db, clock);
    let all = fresh.quotas(None).await.unwrap();
    let item = &all["items"][0];
    assert_eq!(item["limits"]["requests_per_min"]["limit"], 100);
    assert_eq!(item["limits"]["requests_per_min"]["remaining"], 40);
    assert!(item["limits"]["tokens_per_min"]["limit"] == 1000);
    assert!((item["used"].as_f64().unwrap() - 0.6).abs() < 1e-9);
}

#[tokio::test]
async fn quota_session_usage_counts_only_that_session() {
    let (_d, db) = setup().await;
    let repo = QuotaRepo::new(db, Arc::new(ManualClock::new(NOW)));
    repo.record(rec(2, false, None)).await.unwrap();
    let s = repo.quotas(Some(1)).await.unwrap();
    assert_eq!(s["items"][0]["usage"]["period"], "session");
    assert_eq!(s["items"][0]["usage"]["session_id"], 1);
    let none = repo.quotas(Some(777)).await.unwrap();
    assert_eq!(none["items"].as_array().unwrap().len(), 0);
}
