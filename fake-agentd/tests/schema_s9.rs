//! Full schema against db.md, migrations from scratch and from each version, incompatible version.
//!
//! The reference is the DDL from `docs/reference/db.md` (all ```sql blocks) run on an empty DB. Migrations from other
//! sprints (0002 onward) land in `main` with their merge; until they exist, `schema_matches_db_md` is red in substance.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use fake_agentd::{MIGRATIONS, Migration};
use rusqlite::Connection;

fn db_md() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/db.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// All ```sql blocks of the document, in order of appearance.
fn ddl_blocks(md: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), None::<String>);
    for line in md.lines() {
        match (&mut cur, line.trim_end()) {
            (None, "```sql") => cur = Some(String::new()),
            (Some(_), "```") => out.extend(cur.take()),
            (Some(buf), l) => {
                buf.push_str(l);
                buf.push('\n');
            }
            _ => {}
        }
    }
    out
}

fn reference_db() -> Connection {
    let conn = Connection::open_in_memory().expect("memory");
    for block in ddl_blocks(&db_md()) {
        conn.execute_batch(&block)
            .unwrap_or_else(|e| panic!("DDL from db.md does not run: {e}\n{block}"));
    }
    // db.md section 6: `0009_web_token` recreates `api_token` (the block in section 3.13 describes the state after `0001`);
    // the 0009 rules from the document are checked by `api_token_0009_rules_from_db_md`
    let m9 = MIGRATIONS.iter().find(|m| m.version == 9).expect("0009");
    conn.execute_batch(m9.sql)
        .expect("0009 on top of the reference");
    conn
}

fn migrated_db(migrations: &[Migration]) -> (tempfile::TempDir, PathBuf, Connection) {
    let (dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, migrations).expect("migrations"));
    let conn = Connection::open(&path).expect("open");
    (dir, path, conn)
}

fn norm(sql: &str) -> String {
    let no_comments: Vec<&str> = sql
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect();
    no_comments
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        // ALTER TABLE ... RENAME wraps the name in quotes; this does not affect the DDL meaning
        .replace('"', "")
}

fn names(conn: &Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .expect("prepare");
    stmt.query_map([kind], |r| r.get(0))
        .expect("query")
        .flatten()
        .collect()
}

/// Table structure: columns, foreign keys, indexes.
fn structure(conn: &Connection, table: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cols = conn
        .prepare(&format!("PRAGMA table_xinfo('{table}')"))
        .expect("table_xinfo");
    let rows = cols
        .query_map([], |r| {
            Ok(format!(
                "col {} {} notnull={} default={:?} pk={} hidden={}",
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?
            ))
        })
        .expect("rows");
    out.extend(rows.flatten());
    let mut fks = conn
        .prepare(&format!("PRAGMA foreign_key_list('{table}')"))
        .expect("fk");
    let rows = fks
        .query_map([], |r| {
            Ok(format!(
                "fk {}->{}.{:?} on_update={} on_delete={}",
                r.get::<_, String>(3)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?
            ))
        })
        .expect("rows");
    out.extend(rows.flatten());
    let mut idx = conn
        .prepare(&format!("PRAGMA index_list('{table}')"))
        .expect("index_list");
    let list: Vec<(String, i64, String, i64)> = idx
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .expect("rows")
        .flatten()
        .collect();
    for (name, unique, origin, partial) in list {
        let mut info = conn
            .prepare(&format!("PRAGMA index_info('{name}')"))
            .expect("index_info");
        let cols: Vec<String> = info
            .query_map([], |r| r.get::<_, Option<String>>(2))
            .expect("rows")
            .flatten()
            .map(|c| c.unwrap_or_else(|| "<expr>".into()))
            .collect();
        let shown = if origin == "c" { name } else { "<auto>".into() };
        out.push(format!(
            "index {shown} unique={unique} origin={origin} partial={partial} cols={cols:?}"
        ));
    }
    out.sort();
    out
}

fn schema_diff(expected: &Connection, actual: &Connection) -> Vec<String> {
    let mut diff = Vec::new();
    for kind in ["table", "index", "trigger"] {
        let (e, a) = (names(expected, kind), names(actual, kind));
        for n in e.iter().filter(|n| !a.contains(n)) {
            diff.push(format!("missing {kind} {n}"));
        }
        for n in a.iter().filter(|n| !e.contains(n)) {
            diff.push(format!("extra {kind} {n}"));
        }
    }
    for table in names(expected, "table")
        .into_iter()
        .filter(|t| names(actual, "table").contains(t))
    {
        let (se, sa) = (structure(expected, &table), structure(actual, &table));
        if se != sa {
            diff.push(format!("table {table}: expected {se:?}, have {sa:?}"));
        }
    }
    let sql_of = |c: &Connection| -> BTreeMap<String, String> {
        let mut stmt = c
            .prepare("SELECT type || ' ' || name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'")
            .expect("prepare");
        stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, norm(&r.get::<_, String>(1)?)))
        })
        .expect("rows")
        .flatten()
        .collect()
    };
    let (se, sa) = (sql_of(expected), sql_of(actual));
    for (k, v) in &se {
        if sa.get(k).is_some_and(|x| x != v) {
            diff.push(format!("DDL text of {k} differs (CHECK/constraints)"));
        }
    }
    diff
}

#[test]
fn db_md_has_26_tables() {
    let reference = reference_db();
    let tables = names(&reference, "table");
    assert_eq!(tables.len(), 26, "db.md tables: {tables:?}");
}

#[test]
fn schema_matches_db_md() {
    let (_d, _p, actual) = migrated_db(MIGRATIONS);
    let reference = reference_db();
    let diff = schema_diff(&reference, &actual);
    assert!(
        diff.is_empty(),
        "schema after migrations differs from db.md ({} differences):\n{}",
        diff.len(),
        diff.join("\n")
    );
}

fn dump(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT type || ' ' || name, coalesce(sql, '') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY 1")
        .expect("prepare");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("rows")
        .flatten()
        .collect()
}

#[test]
fn migrations_apply_from_scratch() {
    let (_d, _p, conn) = migrated_db(MIGRATIONS);
    let last = MIGRATIONS.last().map_or(0, |m| m.version);
    let want: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
    assert_eq!(applied_versions(&conn), want);
    assert_eq!(user_version(&conn), last);
    let fk: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .expect("fk");
    assert_eq!(fk, 0, "foreign_key_check is empty");
    let ok: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity");
    assert_eq!(ok, "ok");
}

#[test]
fn migrations_from_every_version_equal_clean() {
    let (_d, _p, clean) = migrated_db(MIGRATIONS);
    let expected = dump(&clean);
    assert!(
        MIGRATIONS
            .windows(2)
            .all(|w| w[1].version == w[0].version + 1),
        "migration versions run consecutively without gaps"
    );
    for n in 1..MIGRATIONS.len() {
        let (_d, path, conn) = migrated_db(&MIGRATIONS[..n]);
        assert_eq!(user_version(&conn), MIGRATIONS[n - 1].version);
        drop(conn);
        drop(fake_agentd::Db::open(&path).expect("upgrade from a version"));
        let conn = Connection::open(&path).expect("open");
        assert_eq!(
            dump(&conn),
            expected,
            "a DB from version {} after the upgrade differs from a clean one",
            MIGRATIONS[n - 1].version
        );
        assert_eq!(
            user_version(&conn),
            MIGRATIONS.last().map_or(0, |m| m.version)
        );
    }
}

#[test]
fn migration_files_are_registered_and_forward_only() {
    // 0001, 0002 and 0006..0010 live in agentd/migrations (the single source), the rest in fake-agentd/migrations
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<String> = Vec::new();
    for sub in ["migrations", "../agentd/migrations"] {
        for e in fs::read_dir(root.join(sub)).expect("migrations") {
            let name = e.expect("entry").file_name().to_string_lossy().into_owned();
            if !files.contains(&name) {
                files.push(name);
            }
        }
    }
    files.sort();
    let want: Vec<String> = MIGRATIONS
        .iter()
        .map(|m| format!("{:04}_{}.sql", m.version, m.name))
        .collect();
    // the message has only version numbers: full file names of a long list are taken by the secret scanner for a key
    let unregistered: Vec<&str> = files
        .iter()
        .filter(|f| !want.contains(f))
        .map(|f| &f[..4])
        .collect();
    let missing: Vec<&str> = want
        .iter()
        .filter(|w| !files.contains(w))
        .map(|w| &w[..4])
        .collect();
    assert!(
        unregistered.is_empty() && missing.is_empty(),
        "files without a MIGRATIONS entry: {} {unregistered:?}; entries without a file: {} {missing:?}; there must be no down migrations",
        unregistered.len(),
        missing.len()
    );
    for m in MIGRATIONS {
        let lower = m.sql.to_lowercase();
        assert!(
            !lower.contains("user_version") && !lower.contains("insert into schema_migrations"),
            "{}: user_version and schema_migrations are written only by the runner",
            m.name
        );
    }
}

/// `api_token` rules after 0009 in the words of db.md section 6: `web` only `chat`/`decide`, `decide` only for `web`/`test`,
/// `web` requires `expires_at`; `admin` is not allowed for `web`.
#[test]
fn api_token_0009_rules_from_db_md() {
    let (_d, _p, conn) = migrated_db(MIGRATIONS);
    let insert = |scope: &str, kind: &str, expires: Option<i64>, n: usize| {
        conn.execute(
            "INSERT INTO api_token(token_hash, label, scope, kind, created_at, expires_at) VALUES (?1, 't', ?2, ?3, 10, ?4)",
            rusqlite::params![format!("{n:064x}"), scope, kind, expires],
        )
    };
    assert!(
        insert("decide", "web", Some(99), 1).is_ok(),
        "web + decide with expiry"
    );
    assert!(
        insert("chat", "web", Some(99), 2).is_ok(),
        "web + chat with expiry"
    );
    assert!(
        insert("chat", "web", None, 3).is_err(),
        "web requires expires_at"
    );
    assert!(
        insert("admin", "web", Some(99), 4).is_err(),
        "web does not get admin"
    );
    assert!(
        insert("read", "web", Some(99), 5).is_err(),
        "web only chat/decide"
    );
    assert!(
        insert("decide", "external", None, 6).is_err(),
        "decide only for web/test"
    );
    assert!(
        insert("decide", "test", None, 7).is_ok(),
        "decide is allowed for test"
    );
}
