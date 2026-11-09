//! A3-11 (DB): `ChatRepo` on a real DB: idempotency, length in bytes, `streaming` → `interrupted` transition,
//! history with a budget, attachments.

use std::sync::Arc;

use agentd_core::{ChatError, ChatStore, ManualClock, Posted, UserMessage};
use agentd_db::{ChatRepo, Db, DbConfig};

async fn setup() -> (tempfile::TempDir, Arc<ChatRepo>, Arc<Db>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1),(2,'b',1,1);
             INSERT INTO sessions(id, parent_id, name, created_at, updated_at) VALUES (3,1,'thread',1,1);",
        )
    })
    .await
    .unwrap();
    let repo = Arc::new(ChatRepo::new(
        db.clone(),
        Arc::new(ManualClock::new(1_800_000_000)),
        dir.path().join("agentos"),
    ));
    (dir, repo, db)
}

fn um(cid: &str, text: &str) -> UserMessage {
    UserMessage {
        client_id: cid.into(),
        body_md: text.into(),
        attachment_ids: vec![],
    }
}

#[tokio::test]
async fn message_duplicate_by_client_id_is_never_created_twice() {
    let (_d, repo, db) = setup().await;
    // Ten concurrent sends of one client_id: exactly one creation, the rest are repeats.
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let r = repo.clone();
        tasks.push(tokio::spawn(async move {
            r.post_user(1, um("same", "text")).await.unwrap()
        }));
    }
    let mut created = 0;
    for t in tasks {
        if matches!(t.await.unwrap(), Posted::Created(_)) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    let n: i64 = db
        .read(|c| c.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        repo.post_user(1, um("same", "other")).await,
        Err(ChatError::Conflict("client_id_conflict"))
    );
    assert!(matches!(
        repo.post_user(2, um("same", "text")).await,
        Ok(Posted::Created(_))
    ));
    assert_eq!(
        repo.post_user(99, um("x", "y")).await,
        Err(ChatError::NotFound)
    );
}

#[tokio::test]
async fn stream_append_counts_bytes_and_finish_once() {
    let (_d, repo, _db) = setup().await;
    let m = repo.create_agent(1, "Agent".into()).await.unwrap();
    let id = m["id"].as_i64().unwrap();
    assert_eq!(
        (
            m["status"].as_str(),
            m["body_md"].as_str(),
            m["client_id"].is_null()
        ),
        (Some("streaming"), Some(""), true)
    );
    assert_eq!(
        repo.append(id, "日本語".into()).await,
        Ok(9),
        "length in UTF-8 bytes, not characters"
    );
    assert_eq!(repo.append(id, " 🔑".into()).await, Ok(14));
    assert_eq!(repo.finish(id, "done", None).await, Ok(14));
    assert_eq!(
        repo.finish(id, "error", None).await,
        Err(ChatError::NotFound),
        "a closed message does not change"
    );
    assert_eq!(
        repo.append(id, "more".into()).await,
        Err(ChatError::NotFound),
        "cannot append to a closed message"
    );
    let page = repo.list(1, None, 10).await.unwrap();
    assert_eq!(page[0]["body_md"], "日本語 🔑");
    assert_eq!(page[0]["status"], "done");
}

#[tokio::test]
async fn streaming_become_interrupted_on_start() {
    let (_d, repo, _db) = setup().await;
    let a = repo.create_agent(1, "A".into()).await.unwrap()["id"]
        .as_i64()
        .unwrap();
    let b = repo.create_agent(2, "A".into()).await.unwrap()["id"]
        .as_i64()
        .unwrap();
    repo.append(a, "part".into()).await.unwrap();
    repo.finish(b, "done", None).await.unwrap();
    assert_eq!(repo.interrupt_streaming().await, Ok(1));
    let snap = repo.session_snapshot(1).await.unwrap();
    assert_eq!(snap["messages"][0]["status"], "interrupted");
    assert_eq!(
        snap["messages"][0]["body_md"], "part",
        "accumulated text stays"
    );
    assert_eq!(
        repo.interrupt_streaming().await,
        Ok(0),
        "a repeat changes nothing"
    );
    let other = repo.list(2, None, 10).await.unwrap();
    assert_eq!(other[0]["status"], "done");
}

#[tokio::test]
async fn history_budget_parent_and_status_filter() {
    let (_d, repo, _db) = setup().await;
    let mut ids = Vec::new();
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        let Ok(Posted::Created(m)) = repo.post_user(1, um(&format!("c{i}"), text)).await else {
            panic!()
        };
        ids.push(m["id"].as_i64().unwrap());
        let a = repo.create_agent(1, "A".into()).await.unwrap()["id"]
            .as_i64()
            .unwrap();
        repo.append(a, format!("reply {text}")).await.unwrap();
        repo.finish(a, if i == 1 { "error" } else { "done" }, None)
            .await
            .unwrap();
    }
    let all = repo.history(1, i64::MAX, false, 10_000).await.unwrap();
    let texts: Vec<&str> = all.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(
        texts,
        ["one", "reply one", "two", "three", "reply three"],
        "a reply with status error does not enter the history"
    );
    let tail = repo.history(1, i64::MAX, false, 8).await.unwrap();
    assert_eq!(tail.len(), 1, "budget: at least the last turn");
    assert_eq!(tail[0].text, "reply three");
    let before = repo.history(1, ids[1], false, 10_000).await.unwrap();
    assert_eq!(
        before.last().unwrap().text,
        "reply one",
        "strictly before the cursor"
    );
    // Thread: the parent's tail comes before its own turns.
    let Ok(Posted::Created(t)) = repo.post_user(3, um("t", "in thread")).await else {
        panic!()
    };
    let with = repo
        .history(3, t["id"].as_i64().unwrap() + 1, true, 10_000)
        .await
        .unwrap();
    assert_eq!(with.first().unwrap().text, "one");
    assert_eq!(with.last().unwrap().text, "in thread");
    let without = repo
        .history(3, t["id"].as_i64().unwrap() + 1, false, 10_000)
        .await
        .unwrap();
    assert_eq!(without.len(), 1);
}

#[tokio::test]
async fn attachments_roundtrip_limits_and_session_binding() {
    let (_d, repo, _db) = setup().await;
    let a = repo
        .put_attachment(1, "../../x".into(), "TEXT/Plain".into(), b"abc".to_vec())
        .await
        .unwrap();
    assert_eq!(
        (a.name.as_str(), a.mime.as_str(), a.size),
        ("../../x", "text/plain", 3)
    );
    let bad_mime = repo
        .put_attachment(1, "n\n\0".into(), "not mime".into(), vec![1])
        .await
        .unwrap();
    assert_eq!(
        (bad_mime.name.as_str(), bad_mime.mime.as_str()),
        ("n", "application/octet-stream")
    );
    let (info, data) = repo.get_attachment(a.id).await.unwrap();
    assert_eq!((info, data), (a.clone(), b"abc".to_vec()));
    assert_ne!(a.id, bad_mime.id);
    assert_eq!(repo.get_attachment(777).await, Err(ChatError::NotFound));
    assert_eq!(
        repo.put_attachment(99, "n".into(), "a/b".into(), vec![])
            .await,
        Err(ChatError::NotFound)
    );
    let big = vec![0u8; agentd_core::MAX_ATTACHMENT_BYTES + 1];
    assert_eq!(
        repo.put_attachment(1, "n".into(), "a/b".into(), big).await,
        Err(ChatError::TooLarge)
    );
    let ok = UserMessage {
        client_id: "m".into(),
        body_md: "see".into(),
        attachment_ids: vec![a.id],
    };
    assert!(matches!(
        repo.post_user(1, ok.clone()).await,
        Ok(Posted::Created(_))
    ));
    assert!(matches!(
        repo.post_user(
            2,
            UserMessage {
                client_id: "m2".into(),
                ..ok
            }
        )
        .await,
        Err(ChatError::Invalid(_))
    ));
}
