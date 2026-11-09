//! A queue refusal and a resubscribe error reach the chat (against `fake-agentd` and without it).
mod common;

use std::time::Instant;

use common::{WAIT, client, start};
use controllers::chat_view::{ChatItem, Delivery};
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{Controller, Effect};
use core_client::CoreEvent;
use core_client::api::{ApiError, ErrorCode};
use fake_agentd::Config;
use ui::chat_feed::{resubscribe, route};

fn delivery(c: &ChatController) -> Vec<Delivery> {
    c.view()
        .items
        .iter()
        .filter_map(|i| match i {
            ChatItem::Bubble(b) => Some(b.delivery),
            _ => None,
        })
        .collect()
}

fn pending_chat(client_id: &str) -> ChatController {
    let mut c = ChatController::new();
    let now = Instant::now();
    let _: Vec<Effect> = c.update(ChatMsg::Select(Some(1)), now);
    let _ = c.update(ChatMsg::DraftChanged("hello".into()), now);
    let _ = c.update(
        ChatMsg::Send {
            client_id: client_id.into(),
        },
        now,
    );
    c
}

#[test]
fn route_failed_event_marks_pending() {
    let mut c = pending_chat("cid-1");
    let error = ApiError {
        status: 422,
        code: ErrorCode::ValidationFailed,
        message: String::new(),
        details: serde_json::Value::Null,
        retryable: false,
        request_id: None,
        retry_after: None,
    };
    let ev = CoreEvent::Failed {
        client_id: "cid-1".into(),
        error,
    };
    let _ = route(&mut c, &ev, Instant::now());
    assert_eq!(delivery(&c), vec![Delivery::Failed]);
}

#[tokio::test]
async fn queue_rejection_reaches_chat_controller() {
    let d = start(Config::temp().no_ticker()).await;
    let cl = client(&d);
    let rx = cl.events();
    let mut c = ChatController::new();
    let now = Instant::now();
    let _ = c.update(ChatMsg::Select(Some(99_999)), now);
    let _ = c.update(ChatMsg::DraftChanged("to nowhere".into()), now);
    let _ = c.update(
        ChatMsg::Send {
            client_id: "cid-404".into(),
        },
        now,
    );
    assert!(
        cl.enqueue_message(99_999, "cid-404", "to nowhere", &[])
            .is_ok()
    );
    let end = Instant::now() + WAIT;
    while Instant::now() < end && delivery(&c) != vec![Delivery::Failed] {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
        {
            let _ = route(&mut c, &ev, Instant::now());
        }
    }
    assert_eq!(delivery(&c), vec![Delivery::Failed]);
}

#[tokio::test]
async fn resubscribe_reports_errors() {
    let d = start(Config::temp().no_ticker()).await;
    let cl = client(&d);
    assert!(resubscribe(&cl, "session.1").is_ok());
    assert!(
        resubscribe(&cl, "bad topic").is_err(),
        "the error is not lost"
    );
}
