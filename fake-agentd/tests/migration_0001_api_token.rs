//! `api_token` constraints from migration 0001 (db.md section 3.13).
//! The `migration_0001_api_token_constraints` test.

mod common;

use common::*;
use rusqlite::Connection;

struct Tok<'a> {
    hash: String,
    label: &'a str,
    scope: &'a str,
    kind: &'a str,
    allow: Option<&'a str>,
    created: i64,
    expires: Option<i64>,
}

fn hash(n: u8) -> String {
    format!("{:064x}", u128::from(n) + 1)
}

fn tok(n: u8, scope: &'static str, kind: &'static str) -> Tok<'static> {
    Tok {
        hash: hash(n),
        label: "t",
        scope,
        kind,
        allow: None,
        created: 1,
        expires: None,
    }
}

fn insert(w: &Connection, t: &Tok) -> rusqlite::Result<usize> {
    w.execute(
        "INSERT INTO api_token(token_hash, label, scope, kind, allowlist_json, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![t.hash, t.label, t.scope, t.kind, t.allow, t.created, t.expires],
    )
}

#[test]
fn migration_0001_api_token_constraints() {
    let (_d, path) = temp_db_path();
    let db = open(&path);
    let list = Some("[\"POST /sessions/*/tool-results\"]");

    // all five kinds are accepted; agent and module require a list
    db.write(|w| {
        insert(w, &tok(1, "chat", "gui"))?;
        insert(
            w,
            &Tok {
                expires: Some(100),
                ..tok(2, "read", "external")
            },
        )?;
        insert(w, &tok(3, "admin", "test"))?;
        insert(
            w,
            &Tok {
                allow: list,
                ..tok(4, "chat", "agent")
            },
        )?;
        insert(
            w,
            &Tok {
                allow: Some("[]"),
                ..tok(5, "read", "module")
            },
        )?;
        insert(w, &tok(6, "admin", "gui"))?;
        Ok(())
    })
    .expect("valid tokens are accepted");

    let rejected = |label: &str, t: Tok| {
        let err = db
            .write(|w| insert(w, &t))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err.contains("constraint") || err.contains("api_token"),
            "{label}: a CHECK rejection is expected, got {err:?}"
        );
    };
    rejected("agent without allowlist", tok(10, "chat", "agent"));
    rejected("module without allowlist", tok(11, "chat", "module"));
    rejected("external admin", tok(12, "admin", "external"));
    rejected(
        "agent admin",
        Tok {
            allow: Some("[]"),
            ..tok(13, "admin", "agent")
        },
    );
    rejected(
        "module admin",
        Tok {
            allow: Some("[]"),
            ..tok(14, "admin", "module")
        },
    );
    rejected("foreign kind", tok(15, "chat", "root"));
    rejected("foreign scope", tok(16, "owner", "gui"));
    rejected(
        "short hash",
        Tok {
            hash: "abc".into(),
            ..tok(0, "chat", "gui")
        },
    );
    rejected(
        "uppercase hash",
        Tok {
            hash: "A".repeat(64),
            ..tok(0, "chat", "gui")
        },
    );
    rejected(
        "hash is not hex",
        Tok {
            hash: "g".repeat(64),
            ..tok(0, "chat", "gui")
        },
    );
    rejected("repeated hash", tok(1, "chat", "gui"));
    rejected(
        "empty label",
        Tok {
            label: "",
            ..tok(17, "chat", "gui")
        },
    );
    let long = "ü".repeat(101);
    rejected(
        "label of 101 characters",
        Tok {
            label: &long,
            ..tok(18, "chat", "gui")
        },
    );
    rejected(
        "expires_at not after created_at",
        Tok {
            created: 5,
            expires: Some(5),
            ..tok(19, "chat", "gui")
        },
    );
    rejected(
        "allowlist is not JSON",
        Tok {
            allow: Some("{oops"),
            ..tok(20, "chat", "agent")
        },
    );

    let update_rejected = |label: &str, sql: &str| {
        let err = db
            .write(|w| w.execute(sql, []))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err.contains("api_token"),
            "{label}: a trigger rejection is expected, got {err:?}"
        );
    };
    update_rejected(
        "editing token_hash",
        &format!(
            "UPDATE api_token SET token_hash = '{}' WHERE id = 1",
            hash(30)
        ),
    );
    db.write(|w| w.execute("UPDATE api_token SET revoked_at = 50 WHERE id = 1", []))
        .expect("first revocation");
    update_rejected(
        "editing revoked_at again",
        "UPDATE api_token SET revoked_at = 60 WHERE id = 1",
    );
    update_rejected(
        "clearing the revocation",
        "UPDATE api_token SET revoked_at = NULL WHERE id = 1",
    );
    db.write(|w| {
        w.execute(
            "UPDATE api_token SET revoked_at = 50, last_used_at = 70 WHERE id = 1",
            [],
        )
    })
    .expect("the same revoked_at and last_used_at are allowed");
}
