//! Tree fixtures: personas "Personal" and "Work" with accounts from `0002`, nested folders, sessions, threads, unread.
//! Created at mock start, not by a migration; leaves existing data alone.

use rusqlite::{Connection, params};

pub fn seed_rows(w: &mut Connection, now: i64) -> rusqlite::Result<()> {
    let tx = w.transaction()?;
    let empty: i64 = tx.query_row("SELECT count(*) FROM personas", [], |r| r.get(0))?;
    if empty > 0 {
        return tx.commit();
    }
    let account = |slug: &str| -> Option<i64> {
        tx.query_row(
            "SELECT id FROM accounts WHERE slug = ?1",
            params![slug],
            |r| r.get(0),
        )
        .ok()
    };
    let persona =
        |name: &str, color: &str, model: Option<i64>, style: &str| -> rusqlite::Result<i64> {
            tx.execute(
                "INSERT INTO personas(name, color, model_acc, style) VALUES (?1, ?2, ?3, ?4)",
                params![name, color, model, style],
            )?;
            Ok(tx.last_insert_rowid())
        };
    let home = persona("Personal", "#e64980", account("ol1"), "friendly")?;
    let work = persona("Work", "#1c7ed6", account("cl1"), "to the point")?;
    for (p, slugs) in [(home, &["ol1"][..]), (work, &["cl1", "cl2"][..])] {
        for slug in slugs {
            if let Some(a) = account(slug) {
                tx.execute(
                    "INSERT INTO persona_accounts(persona_id, account_id) VALUES (?1, ?2)",
                    params![p, a],
                )?;
            }
        }
    }
    let folder = |parent: Option<i64>, name: &str, persona: Option<i64>| -> rusqlite::Result<i64> {
        tx.execute(
            "INSERT INTO folders(parent_id, name, persona_id, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![parent, name, persona, now],
        )?;
        Ok(tx.last_insert_rowid())
    };
    let clients = folder(None, "Clients", Some(work))?;
    let projects = folder(Some(clients), "Projects", None)?;
    let house = folder(None, "Home", Some(home))?;
    let session = |folder: Option<i64>,
                   parent: Option<i64>,
                   name: &str,
                   persona: Option<i64>,
                   path: Option<&str>,
                   unread: i64|
     -> rusqlite::Result<i64> {
        tx.execute(
            "INSERT INTO sessions(folder_id, parent_id, name, persona_id, path, unread, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![folder, parent, name, persona, path, unread, now],
        )?;
        Ok(tx.last_insert_rowid())
    };
    let release = session(
        Some(projects),
        None,
        "Release 1.2",
        None,
        Some("/home/user/code/release"),
        3,
    )?;
    session(Some(projects), Some(release), "Review", None, None, 0)?;
    session(Some(projects), Some(release), "Hotfix", None, None, 1)?;
    session(Some(clients), None, "Client emails", None, None, 0)?;
    session(Some(house), None, "Vacation", None, None, 2)?;
    session(None, None, "Notes", Some(home), None, 0)?;
    session(None, None, "No folder", None, None, 0)?;
    tx.commit()
}
