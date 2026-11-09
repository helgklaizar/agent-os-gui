//! A2-07: `TreeStore::bump_unread` is an internal interface for A3-08 (an agent message in an inactive session).

mod common;

use std::sync::{Arc, Mutex};

use agentd_core::{Notifier, SystemClock, TreeError, TreeStore};
use agentd_db::{Db, DbConfig, TreeRepo};
use serde_json::{Value, json};

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl Notifier for Recorder {
    fn publish(&self, topic: &str, data: Value) {
        self.0.lock().unwrap().push((topic.to_string(), data));
    }
}

#[tokio::test]
async fn bump_unread_counts_and_publishes_once_per_bump() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
            [],
        )
    })
    .await
    .unwrap();
    let rec = Arc::new(Recorder::default());
    let repo = TreeRepo::new(db, Arc::new(SystemClock), rec.clone(), None);
    repo.bump_unread(1).await.unwrap();
    repo.bump_unread(1).await.unwrap();
    assert_eq!(repo.get_session(1).await.unwrap()["unread"], 2);
    let got = rec.0.lock().unwrap().clone();
    assert_eq!(
        got,
        vec![
            (
                "sessions".to_string(),
                json!({"op": "unread", "session_id": 1, "unread": 1})
            ),
            (
                "sessions".to_string(),
                json!({"op": "unread", "session_id": 1, "unread": 2})
            ),
        ]
    );
    assert_eq!(repo.bump_unread(99).await, Err(TreeError::NotFound));
    repo.mark_read(1).await.unwrap();
    repo.mark_read(1).await.unwrap();
    assert_eq!(
        rec.0.lock().unwrap().len(),
        3,
        "a repeated read sends no delta"
    );
}
