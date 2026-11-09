//! Migration 0002: indicator state, `accounts` and `quotas` constraints, cascades.

mod common;
mod mig0002_common;

use common::*;
use mig0002_common::*;
use rusqlite::{Connection, params};

#[test]
fn indicators_state_columns() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| w.execute("INSERT INTO indicators(id) VALUES ('cpu')", []))
        .expect("default indicator");
    let (state, seq, changed): (String, i64, Option<i64>) = db
        .reader()
        .unwrap()
        .query_row(
            "SELECT last_state, transition_seq, last_change_at FROM indicators WHERE id = 'cpu'",
            [],
            |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)),
        )
        .unwrap_or_default();
    assert_eq!(state, "unknown");
    assert_eq!(seq, 0);
    assert_eq!(changed, None);
    for good in ["ok", "warn", "bad", "nodata", "unknown"] {
        let ok = db.write(|w| {
            w.execute(
                "UPDATE indicators SET last_state = ?1 WHERE id = 'cpu'",
                params![good],
            )
        });
        assert!(ok.is_ok(), "{good} must be accepted");
    }
    let bad = db.write(|w| {
        w.execute(
            "UPDATE indicators SET last_state = 'red' WHERE id = 'cpu'",
            [],
        )
    });
    assert!(
        bad.is_err(),
        "last_state outside ok/warn/bad/nodata/unknown"
    );
}

#[test]
fn accounts_slug_unique() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| account(w, "cl1")).expect("first account");
    assert!(
        db.write(|w| account(w, "cl1")).is_err(),
        "duplicate slug accepted"
    );
    assert!(db.write(|w| account(w, "cl2")).is_ok());
}

#[test]
fn accounts_status_check() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    for good in ["connected", "disconnected", "qr", "error"] {
        let slug = format!("s-{good}");
        let r = db.write(|w| {
            w.execute(
                "INSERT INTO accounts(slug, type, label, status, created_at) VALUES (?1, 'wa', 'x', ?2, 1)",
                params![slug, good],
            )
        });
        assert!(r.is_ok(), "{good} must be accepted");
    }
    let bad = db.write(|w| {
        w.execute(
            "INSERT INTO accounts(slug, type, label, status, created_at) VALUES ('s-bad', 'wa', 'x', 'online', 1)",
            [],
        )
    });
    assert!(
        bad.is_err(),
        "status outside connected/disconnected/qr/error"
    );
}

#[test]
fn quota_cascade_on_account_delete() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| {
        account(w, "cl1")?;
        account(w, "cl2")?;
        for id in [1, 2] {
            w.execute(
                "INSERT INTO quotas(account_id, kind, updated_at) VALUES (?1, 'api_key', 1)",
                params![id],
            )?;
            w.execute(
                "INSERT INTO quota_samples(account_id, ts, used) VALUES (?1, 1, 0.1)",
                params![id],
            )?;
        }
        w.execute("DELETE FROM accounts WHERE id = 1", [])
    })
    .expect("account deletion");
    let r = db.reader().expect("reader");
    let count = |sql: &str| -> i64 { r.query_row(sql, [], |x| x.get(0)).unwrap_or(-1) };
    assert_eq!(count("SELECT count(*) FROM quotas WHERE account_id = 1"), 0);
    assert_eq!(
        count("SELECT count(*) FROM quota_samples WHERE account_id = 1"),
        0
    );
    assert_eq!(count("SELECT count(*) FROM quotas WHERE account_id = 2"), 1);
    assert_eq!(
        count("SELECT count(*) FROM quota_samples WHERE account_id = 2"),
        1
    );
}

#[test]
fn quotas_one_row_per_account() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| account(w, "cl1")).expect("account");
    let ins = |w: &mut Connection| {
        w.execute(
            "INSERT INTO quotas(account_id, kind, updated_at) VALUES (1, 'api_key', 1)",
            [],
        )
    };
    db.write(ins).expect("first row");
    assert!(
        db.write(ins).is_err(),
        "a second row of the same account was accepted"
    );
}

#[test]
fn quotas_nullable_unknown_limit() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| {
        account(w, "cl1")?;
        account(w, "cl2")?;
        account(w, "cl3")?;
        w.execute(
            "INSERT INTO quotas(account_id, kind, window_h, used, rate, updated_at) VALUES (1, 'api_key', NULL, NULL, NULL, 1)",
            [],
        )
    })
    .expect("limit unknown: NULL is allowed");
    let too_much = db.write(|w| {
        w.execute(
            "INSERT INTO quotas(account_id, kind, used, updated_at) VALUES (2, 'api_key', 1.2, 1)",
            [],
        )
    });
    assert!(too_much.is_err(), "used outside 0..1 accepted");
    let negative = db.write(|w| {
        w.execute(
            "INSERT INTO quotas(account_id, kind, used, updated_at) VALUES (2, 'api_key', -0.1, 1)",
            [],
        )
    });
    assert!(negative.is_err(), "used < 0 accepted");
    let bad_kind = db.write(|w| {
        w.execute(
            "INSERT INTO quotas(account_id, kind, updated_at) VALUES (3, 'subscription', 1)",
            [],
        )
    });
    assert!(bad_kind.is_err(), "kind outside api_key/local accepted");
    let bad_remaining = db.write(|w| {
        w.execute(
            "INSERT INTO quotas(account_id, kind, remaining, updated_at) VALUES (3, 'api_key', 1.5, 1)",
            [],
        )
    });
    assert!(bad_remaining.is_err(), "remaining outside 0..1 accepted");
}

#[test]
fn quotas_local_load() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| {
        account(w, "ol1")?;
        account(w, "ol2")?;
        w.execute(
            "INSERT INTO quotas(account_id, kind, window_h, load, updated_at) VALUES (1, 'local', 0, 0.22, 1)",
            [],
        )
    })
    .expect("local model with load");
    for load in [1.01, -0.01] {
        let r = db.write(|w| {
            w.execute(
                "INSERT INTO quotas(account_id, kind, window_h, load, updated_at) VALUES (2, 'local', 0, ?1, 1)",
                params![load],
            )
        });
        assert!(r.is_err(), "load {load} outside 0..1 accepted");
    }
}
