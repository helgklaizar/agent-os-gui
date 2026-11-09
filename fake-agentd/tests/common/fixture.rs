//! Search fixture: sessions, message, mail, chat, memory fact. The runner creates the schema, only rows are here.
#![allow(dead_code)]

use fake_agentd::Db;

/// Search fixture: sessions, message, mail, chat, memory fact.
pub fn seed_search(db: &Db) {
    db.write(|w| {
        w.execute_batch(
            "INSERT INTO accounts(id, slug, type, label, created_at) VALUES (1, 'gm1', 'gmail', 'Mail', 1), (2, 'wa1', 'wa', 'WhatsApp', 1);
                          INSERT INTO personas(id, name, color) VALUES (1, 'Dev', '#fff');
             INSERT INTO sessions(id, name, created_at, updated_at) VALUES
                              (1, 'Startup: release', 1, 1), (2, 'Server backup', 1, 1);
             INSERT INTO messages(session_id, role, body_md, ts) VALUES
                              (1, 'user', 'discussed memory migration in the release', 10),
                              (2, 'agent', 'backup finished', 11);
             INSERT INTO gw_messages(account_id, ext_id, direction, body, ts) VALUES
                              (1, 'm-1', 'in', 'Hosting invoice under contract 42', 20),
                              (2, 'w-1', 'in', 'hello from whatsapp', 21);",
        )
    })
    .expect("seed");
    // The facts table is created by migration 0008; without it the row is not inserted and the checks fail on an assertion.
    let has_facts = db.reader().ok().and_then(|r| {
        r.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_facts'",
            [],
            |x| x.get::<_, i64>(0),
        )
        .ok()
    }) == Some(1);
    if has_facts {
        db.write(|w| {
            w.execute(
                "INSERT INTO memory_facts(persona_id, session_id, text, source, ts) VALUES (NULL, NULL, 'Owner likes short answers', 'user', 30)",
                [],
            )
        })
        .expect("seed memory");
    }
}
