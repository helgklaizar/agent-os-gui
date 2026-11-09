//! W6 (DB): migration 0009 `web_token`: kind `web`, scope `decide`, `api_token` recreation without data loss.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::migrated_to;
use rusqlite::{Connection, params};

const T0: i64 = 1_800_000_000;

type Row = (i64, String, String, Option<i64>, Option<String>);

fn hash(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn insert(
    c: &Connection,
    n: u8,
    scope: &str,
    kind: &str,
    exp: Option<i64>,
    list: Option<&str>,
) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO api_token(token_hash,label,scope,kind,allowlist_json,created_at,expires_at) VALUES (?1,'t',?2,?3,?4,?5,?6)",
        params![hash(n), scope, kind, list, T0, exp],
    )
}

#[test]
fn rows_triggers_and_index_survive_the_rebuild() {
    let c = migrated_to(8);
    insert(&c, 1, "admin", "gui", None, None).unwrap();
    insert(&c, 2, "chat", "external", Some(T0 + 10), None).unwrap();
    insert(
        &c,
        3,
        "read",
        "module",
        Some(T0 + 10),
        Some("[\"GET /ws\"]"),
    )
    .unwrap();
    c.execute(
        "UPDATE api_token SET revoked_at = ?1 WHERE id = 2",
        [T0 + 5],
    )
    .unwrap();
    let mut c = c;
    migrate(&mut c, &MIGRATIONS[..9], &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 9);
    let rows: Vec<Row> = c
        .prepare("SELECT id, scope, kind, revoked_at, allowlist_json FROM api_token ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_str()), ("admin", "gui"));
    assert_eq!(rows[1].3, Some(T0 + 5));
    assert_eq!(rows[2].4.as_deref(), Some("[\"GET /ws\"]"));
    let fk: i64 = c
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(fk, 0);
    let objs: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE tbl_name = 'api_token' AND type IN ('index','trigger') AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        objs,
        [
            "idx_api_token_active",
            "trg_api_token_hash_immutable",
            "trg_api_token_no_unrevoke"
        ]
    );
    // Triggers work after the recreation too: the hash and revocation are irreversible.
    assert!(
        c.execute(
            "UPDATE api_token SET token_hash = ?1 WHERE id = 1",
            [hash(9)]
        )
        .is_err()
    );
    assert!(
        c.execute("UPDATE api_token SET revoked_at = NULL WHERE id = 2", [])
            .is_err()
    );
    // Autoincrement continues rather than resetting.
    insert(&c, 7, "chat", "external", Some(T0 + 10), None).unwrap();
    let id: i64 = c
        .query_row("SELECT max(id) FROM api_token", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, 4);
}

#[test]
fn web_and_decide_constraints() {
    let c = migrated_to(9);
    let exp = Some(T0 + 100);
    // Allowed.
    insert(&c, 1, "chat", "web", exp, None).unwrap();
    insert(&c, 2, "decide", "web", exp, None).unwrap();
    insert(&c, 3, "decide", "test", None, None).unwrap();
    // Not allowed: a web token is neither `admin` nor `read`, has no expiry; `decide` for other kinds.
    for (n, scope, kind, e) in [
        (10, "admin", "web", exp),
        (11, "read", "web", exp),
        (12, "decide", "web", None),
        (13, "chat", "web", None),
        (14, "decide", "external", exp),
        (15, "decide", "gui", None),
        (16, "decide", "agent", exp),
        (17, "decide", "module", exp),
        (18, "owner", "web", exp),
    ] {
        let list = matches!(kind, "agent" | "module").then_some("[\"GET /ws\"]");
        assert!(
            insert(&c, n, scope, kind, e, list).is_err(),
            "{scope}/{kind}/{e:?} must be rejected"
        );
    }
    // The earlier rules are in place: external is not `admin`, agent without a list is not allowed.
    assert!(insert(&c, 20, "admin", "external", exp, None).is_err());
    assert!(insert(&c, 21, "chat", "agent", exp, None).is_err());
    assert!(insert(&c, 22, "admin", "gui", None, None).is_ok());
}
