//! A3-13: overall reply limit `ChatConfig::total_timeout`: a hung provider yields `error`/`timeout` and does not hold the queue.
//! In-process, on a real DB and a real `ChatService`: the `agentd` binary uses the default limit (10 minutes).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agentd_core::{
    AccountInput, ChatConfig, ChatService, ChatStore, ModelAccount, ModelProvider, ModelRequest,
    Notifier, PortFuture, ProviderError, ProviderEvent, ProviderKind, ProviderRegistry,
    SessionInput, SettingsStore, SystemClock, TreeStore, UserMessage,
};
use agentd_db::{ChatRepo, Db, DbConfig, SettingsRepo, TreeRepo};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Provider: one delta, then silence for an hour.
struct Hang;

impl ModelProvider for Hang {
    fn stream<'a>(
        &'a self,
        _req: ModelRequest,
        tx: mpsc::Sender<ProviderEvent>,
    ) -> PortFuture<'a, ()> {
        Box::pin(async move {
            drop(tx.send(ProviderEvent::Delta("Thinking… ".into())).await);
            tokio::time::sleep(Duration::from_secs(3600)).await;
        })
    }

    fn check<'a>(&'a self, _a: &'a ModelAccount) -> PortFuture<'a, Result<(), ProviderError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Notifier for Events {
    fn publish(&self, topic: &str, data: Value) {
        self.0.lock().unwrap().push((topic.into(), data));
    }
}

fn msgs(page: &Value) -> Vec<Value> {
    page.as_array()
        .or_else(|| page["items"].as_array())
        .expect("messages page")
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_total_timeout_ends_answer_with_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    let clock = Arc::new(SystemClock);
    let events = Arc::new(Events::default());
    let chat = Arc::new(ChatRepo::new(
        db.clone(),
        clock.clone(),
        dir.path().join("agentos"),
    ));
    let tree = Arc::new(TreeRepo::new(
        db.clone(),
        clock.clone(),
        events.clone(),
        None,
    ));
    let settings = Arc::new(SettingsRepo::new(db.clone(), clock));

    let acc = tree
        .create_account(AccountInput {
            kind: Some("scripted".into()),
            label: Some("test".into()),
        })
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    settings
        .patch(
            "models",
            json!({"accounts": {acc.to_string(): {"model": "hang"}}}),
            0,
        )
        .await
        .unwrap();
    let s = tree
        .create_session(SessionInput {
            name: Some("chat".into()),
            ..SessionInput::default()
        })
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    tree.patch_session(
        s,
        SessionInput {
            cfg: json!({"model": acc}).as_object().cloned(),
            ..SessionInput::default()
        },
    )
    .await
    .unwrap();

    let cfg = ChatConfig {
        total_timeout: Duration::from_millis(300),
        ..ChatConfig::default()
    };
    let service = ChatService::new(
        chat.clone(),
        tree,
        settings,
        ProviderRegistry::new().with(ProviderKind::Scripted, Arc::new(Hang)),
        None,
        events,
        cfg,
    );
    let started = Instant::now();
    service
        .send(
            s,
            UserMessage {
                client_id: "t1".into(),
                body_md: "question".into(),
                attachment_ids: vec![],
            },
        )
        .await
        .unwrap();

    let agent = loop {
        let page = msgs(&chat.list(s, None, 10).await.unwrap());
        if let Some(m) = page
            .into_iter()
            .find(|m| m["role"] == "agent" && m["status"] != "streaming")
        {
            break m;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "reply not closed by total_timeout"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "closed before the limit: {:?}",
        started.elapsed()
    );
    assert_eq!(agent["status"], "error", "{agent}");
    assert_eq!(agent["meta"]["error"]["code"], "timeout", "{agent}");
    assert_eq!(agent["meta"]["error"]["retryable"], true, "{agent}");
    assert_eq!(
        agent["body_md"], "Thinking… ",
        "accepted part of the text is kept"
    );
}
