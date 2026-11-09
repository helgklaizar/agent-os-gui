//! A2-01 and A2-06 (DB): migration 0003 `sessions` (db.md sections 3.1, 3.4, section 5) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{migrated_to, names, normalized_schema, repo_root};
use rusqlite::Connection;

fn db3() -> Connection {
    let c = migrated_to(3);
    assert_eq!(user_version(&c).unwrap(), 3, "DB brought up to version 3");
    c
}

fn folder(c: &Connection, parent: Option<i64>) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO folders(parent_id, name, created_at) VALUES (?1, 'f', 1)",
        [parent],
    )?;
    Ok(c.last_insert_rowid())
}

fn session(c: &Connection, parent: Option<i64>) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO sessions(parent_id, name, created_at, updated_at) VALUES (?1, 's', 1, 1)",
        [parent],
    )?;
    Ok(c.last_insert_rowid())
}

fn msg(r: rusqlite::Result<impl std::fmt::Debug>) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn migration_0003_applies_up() {
    let mut c = migrated_to(2);
    assert_eq!(user_version(&c).unwrap(), 2);
    migrate(&mut c, &MIGRATIONS[..3], &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 3);
    let v: i64 = c
        .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v, 3);
    let name: String = c
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 3",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "sessions");
}

#[test]
fn migration_0002_0003_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = common::file_conn(dir.path());
    let opts = MigrateOptions::default();
    migrate(&mut c, &MIGRATIONS[..3], &opts).unwrap();
    assert_eq!(user_version(&c).unwrap(), 3);
    let stamp = |c: &Connection| -> Vec<i64> {
        c.prepare("SELECT applied_at FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = stamp(&c);
    let out = migrate(&mut c, &MIGRATIONS[..3], &opts).unwrap();
    assert!(out.applied.is_empty(), "a repeated start applies nothing");
    assert_eq!(user_version(&c).unwrap(), 3);
    assert_eq!(stamp(&c), before);
}

#[test]
fn migration_0003_schema() {
    let c = db3();
    let tables = names(&c, "table");
    for t in ["personas", "persona_accounts", "folders", "sessions"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    let idx = names(&c, "index");
    for i in [
        "idx_sessions_folder",
        "idx_sessions_parent",
        "idx_folders_parent",
    ] {
        assert!(idx.contains(&i.to_string()), "missing index {i}");
    }
    let trg = names(&c, "trigger");
    for t in [
        "trg_folders_no_cycle",
        "trg_folders_depth_ins",
        "trg_folders_depth_upd",
        "trg_sessions_thread_ins",
        "trg_sessions_thread_upd",
    ] {
        assert!(trg.contains(&t.to_string()), "missing trigger {t}");
    }
    assert!(
        !trg.contains(&"trg_sessions_cleanup".to_string()),
        "it arrives with 0004"
    );
    // CHECK constraints.
    let bad = |sql: &str| c.execute(sql, []).is_err();
    assert!(bad(
        "INSERT INTO personas(name,color,mem_mode) VALUES ('p','#fff','x')"
    ));
    assert!(bad(
        "INSERT INTO personas(name,color,notif_mode) VALUES ('p','#fff','x')"
    ));
    assert!(bad(
        "INSERT INTO folders(id,parent_id,name,created_at) VALUES (7,7,'f',1)"
    ));
    assert!(bad(
        "INSERT INTO sessions(id,parent_id,name,created_at,updated_at) VALUES (7,7,'s',1,1)"
    ));
}

#[test]
fn migration_0003_foreign_keys() {
    let c = db3();
    c.execute_batch(
        "INSERT INTO accounts(slug,type,label,created_at) VALUES ('cl1','anthropic','k',1);
         INSERT INTO personas(name,color,model_acc) VALUES ('p','#fff',1);
         INSERT INTO persona_accounts VALUES (1,1);
         INSERT INTO folders(name,persona_id,created_at) VALUES ('f',1,1);
         INSERT INTO sessions(folder_id,persona_id,name,created_at,updated_at) VALUES (1,1,'s',1,1);",
    )
    .unwrap();
    let n: i64 = c
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn migration_0003_integrity() {
    let c = db3();
    let r: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(r, "ok");
}

#[test]
fn schema_parity_0003_with_fake_agentd() {
    let root = repo_root();
    let fake = root.join("fake-agentd/migrations/0003_sessions.sql");
    if !fake.exists() {
        // The fake-agentd catalog is out of scope for A2: the comparison turns on once 0003 appears there (request in the A2 report).
        eprintln!("fake-agentd/migrations/0003_sessions.sql missing: comparison skipped");
        return;
    }
    let pair = |dir: &str, names: &[&str]| {
        names
            .iter()
            .map(|n| {
                std::fs::read_to_string(root.join(dir).join(n))
                    .or_else(|_| std::fs::read_to_string(root.join("agentd/migrations").join(n)))
                    .unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let files = ["0001_init.sql", "0002_bar.sql", "0003_sessions.sql"];
    assert_eq!(
        normalized_schema(&pair("agentd/migrations", &files)),
        normalized_schema(&pair("fake-agentd/migrations", &files))
    );
}

#[test]
fn folder_cycle_aborts() {
    let c = db3();
    let a = folder(&c, None).unwrap();
    let b = folder(&c, Some(a)).unwrap();
    let d = folder(&c, Some(b)).unwrap();
    let m = msg(c.execute("UPDATE folders SET parent_id = ?1 WHERE id = ?1", [a]));
    assert!(m.contains("folder cycle") || m.contains("CHECK"), "{m}");
    let m = msg(c.execute("UPDATE folders SET parent_id = ?1 WHERE id = ?2", [d, a]));
    assert!(m.contains("folder cycle"), "{m}");
}

#[test]
fn folder_depth_limit_8() {
    let c = db3();
    let mut parent = None;
    for _ in 0..8 {
        parent = Some(folder(&c, parent).unwrap());
    }
    let m = msg(folder(&c, parent));
    assert!(m.contains("folder depth"), "ninth level: {m}");
    // Moving a branch deeper than 8 is also rejected.
    let lone = folder(&c, None).unwrap();
    let m = msg(c.execute(
        "UPDATE folders SET parent_id = ?1 WHERE id = ?2",
        [parent.unwrap(), lone],
    ));
    assert!(m.contains("folder depth"), "{m}");
}

#[test]
fn thread_depth_limit_1() {
    let c = db3();
    let s = session(&c, None).unwrap();
    let t = session(&c, Some(s)).unwrap();
    let m = msg(session(&c, Some(t)));
    assert!(m.contains("thread depth"), "insert: {m}");
    let other = session(&c, None).unwrap();
    let m = msg(c.execute(
        "UPDATE sessions SET parent_id = ?1 WHERE id = ?2",
        [t, other],
    ));
    assert!(m.contains("thread depth"), "update: {m}");
}

#[test]
fn session_delete_cascades_threads() {
    let c = db3();
    let s = session(&c, None).unwrap();
    session(&c, Some(s)).unwrap();
    session(&c, Some(s)).unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    let n: i64 = c
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "threads deleted along with the session");
}

#[test]
fn ids_not_reused_autoincrement() {
    let c = db3();
    let first = session(&c, None).unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [first])
        .unwrap();
    let second = session(&c, None).unwrap();
    assert!(second > first, "{second} must be greater than {first}");
    let f1 = folder(&c, None).unwrap();
    c.execute("DELETE FROM folders WHERE id = ?1", [f1])
        .unwrap();
    assert!(folder(&c, None).unwrap() > f1);
}
