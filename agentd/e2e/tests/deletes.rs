//! A2-06: deletes and cascades per docs/reference/db.md section 5: session, thread, folder, persona, account.

mod common;

use common::{Env, persona};
use serde_json::{Value, json};

fn bookmarks(c: &agentd_db::Connection) -> agentd_db::SqlResult<()> {
    c.execute_batch(
        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES
           ('browser.bookmarks.1', '[]', 1, 1), ('browser.bookmarks.2', '[]', 1, 1);",
    )
}

fn fk_clean(c: &agentd_db::Connection) -> agentd_db::SqlResult<i64> {
    c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
        r.get(0)
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_cascades_threads() {
    let mut e = Env::start().await;
    let s = e.chat.make("/sessions", json!({"name": "s"})).await;
    let keep = e.chat.make("/sessions", json!({"name": "keep"})).await;
    let t1 = e
        .chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t1"}),
        )
        .await;
    e.chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t2"}),
        )
        .await;
    let (st, body) = e.chat.delete(&format!("/sessions/{}", s["id"])).await;
    assert_eq!((st, body), (204, Value::Null));
    let (_, list) = e.read.get("/sessions").await;
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "only keep remains: {list}"
    );
    assert_eq!(list[0]["id"], keep["id"]);
    assert_eq!(
        e.read.get(&format!("/sessions/{}", t1["id"])).await.0,
        404,
        "thread deleted"
    );
    assert_eq!(
        e.chat.delete(&format!("/sessions/{}", s["id"])).await.0,
        404
    );
    assert_eq!(e.inspect(fk_clean).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_delete_leaves_parent() {
    let e = Env::start().await;
    let s = e.chat.make("/sessions", json!({"name": "s"})).await;
    let t = e
        .chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t"}),
        )
        .await;
    assert_eq!(
        e.chat.delete(&format!("/sessions/{}", t["id"])).await.0,
        204
    );
    assert_eq!(e.read.get(&format!("/sessions/{}", s["id"])).await.0, 200);
    assert_eq!(e.read.get(&format!("/sessions/{}", t["id"])).await.0, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_removes_bookmarks_setting() {
    let mut e = Env::start_with(|c| {
        bookmarks(c)?;
        c.execute_batch(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s1', 1, 1), ('s2', 1, 1);",
        )
    })
    .await;
    assert_eq!(e.chat.delete("/sessions/1").await.0, 204);
    let keys: Vec<String> = e
        .inspect(|c| {
            c.prepare("SELECT key FROM settings WHERE key LIKE 'browser.bookmarks.%' ORDER BY key")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(
        keys,
        ["browser.bookmarks.2"],
        "bookmarks of the deleted session are gone, others remain"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_foreign_keys_clean_and_ids_not_reused() {
    let mut e = Env::start().await;
    let p = e.admin.make("/personas", persona("p")).await;
    let f = e
        .chat
        .make("/folders", json!({"name": "f", "persona_id": p["id"]}))
        .await;
    let s = e
        .chat
        .make("/sessions", json!({"name": "s", "folder_id": f["id"]}))
        .await;
    e.chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t"}),
        )
        .await;
    e.chat.delete(&format!("/sessions/{}", s["id"])).await;
    e.chat.delete(&format!("/folders/{}", f["id"])).await;
    let again = e.chat.make("/sessions", json!({"name": "new"})).await;
    assert!(
        again["id"].as_i64() > s["id"].as_i64(),
        "id is not reused: {again}"
    );
    let n = e
        .inspect(|c| {
            Ok((
                fk_clean(c)?,
                c.query_row(
                    "SELECT count(*) FROM sessions WHERE parent_id IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0),
                )?,
            ))
        })
        .await;
    assert_eq!(n, (0, 0), "no orphans");
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_delete_moves_content_up() {
    let mut e = Env::start().await;
    let root = e.chat.make("/folders", json!({"name": "root"})).await;
    let mid = e
        .chat
        .make("/folders", json!({"name": "mid", "parent_id": root["id"]}))
        .await;
    let sub = e
        .chat
        .make("/folders", json!({"name": "sub", "parent_id": mid["id"]}))
        .await;
    let s = e
        .chat
        .make("/sessions", json!({"name": "s", "folder_id": mid["id"]}))
        .await;
    let t = e
        .chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t"}),
        )
        .await;
    assert_eq!(
        e.chat.delete(&format!("/folders/{}", mid["id"])).await.0,
        204
    );
    let (_, folders) = e.read.get("/folders").await;
    let ids: Vec<&Value> = folders
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["id"])
        .collect();
    assert_eq!(ids, [&root["id"], &sub["id"]], "row deleted: {folders}");
    assert_eq!(
        folders[1]["parent_id"], root["id"],
        "nested folder moves to its parent"
    );
    let (_, s_after) = e.read.get(&format!("/sessions/{}", s["id"])).await;
    assert_eq!(
        s_after["folder_id"], root["id"],
        "session moves to the deleted folder's parent"
    );
    let (_, t_after) = e.read.get(&format!("/sessions/{}", t["id"])).await;
    assert_eq!(
        t_after["folder_id"], root["id"],
        "thread stays with its parent"
    );
    // Root folder: contents move to the root.
    e.chat.delete(&format!("/folders/{}", root["id"])).await;
    let (_, s_root) = e.read.get(&format!("/sessions/{}", s["id"])).await;
    assert_eq!(s_root["folder_id"], Value::Null);
    let (_, folders) = e.read.get("/folders").await;
    assert_eq!(folders[0]["parent_id"], Value::Null);
    assert_eq!(e.inspect(fk_clean).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_soft_delete_clears_refs() {
    let mut e = Env::start_with(|c| {
        c.execute_batch(
            "INSERT INTO accounts(slug, type, label, created_at) VALUES ('cl1', 'anthropic', 'k', 1);",
        )
    })
    .await;
    let p = e
        .admin
        .make(
            "/personas",
            json!({"name": "p", "color": "#fff", "account_ids": [1]}),
        )
        .await;
    let keep = e.admin.make("/personas", persona("keep")).await;
    e.chat
        .make("/folders", json!({"name": "f", "persona_id": p["id"]}))
        .await;
    let s = e
        .chat
        .make("/sessions", json!({"name": "s", "persona_id": p["id"]}))
        .await;
    let s2 = e
        .chat
        .make("/sessions", json!({"name": "s2", "persona_id": keep["id"]}))
        .await;
    let (_, page) = e.admin.get("/settings/personas").await;
    let pid = p["id"].as_i64().unwrap();
    let balance = json!({"balance": {"priority_persona_id": pid, "fallback": "local"}});
    let (st, v) = e
        .admin
        .patch_if_match("/settings/personas", balance, page["rev"].as_i64().unwrap())
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(e.admin.delete(&format!("/personas/{pid}")).await.0, 204);
    let (_, f_after) = e.read.get("/folders").await;
    assert_eq!(f_after[0]["persona_id"], Value::Null, "{f_after}");
    assert_eq!(
        e.read.get(&format!("/sessions/{}", s["id"])).await.1["persona_id"],
        Value::Null
    );
    assert_eq!(
        e.read.get(&format!("/sessions/{}", s2["id"])).await.1["persona_id"],
        keep["id"],
        "foreign persona is untouched"
    );
    let (_, page) = e.read.get("/settings/personas").await;
    assert_eq!(
        page["balance"]["priority_persona_id"],
        Value::Null,
        "priority reset: {page}"
    );
    assert_eq!(
        page["balance"]["fallback"], "local",
        "the rest of the balance is untouched"
    );
    let row: (bool, i64, i64) = e
        .inspect(move |c| {
            Ok((
                c.query_row(
                    "SELECT deleted_at IS NOT NULL FROM personas WHERE id = ?1",
                    [pid],
                    |r| r.get(0),
                )?,
                c.query_row(
                    "SELECT count(*) FROM persona_accounts WHERE persona_id = ?1",
                    [pid],
                    |r| r.get(0),
                )?,
                fk_clean(c)?,
            ))
        })
        .await;
    assert_eq!(row, (true, 0, 0));
}
