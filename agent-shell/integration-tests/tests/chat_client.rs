//! Core client against `fake-agentd` over a real socket: send, stream, stop, queue on a WS drop.

mod common;

use common::{WAIT, client, http, start};
use core_client::chat::SendOutcome;
use core_client::{CoreEvent, HttpClient};
use fake_agentd::Config;
use serde_json::{Value, json};

const SESSION: i64 = 1;

fn uuid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn history(h: &HttpClient) -> Vec<Value> {
    let mut v = h
        .list_messages(SESSION, None, 200)
        .await
        .unwrap_or_default();
    v.reverse();
    v
}

async fn wait_done(rx: &async_channel::Receiver<CoreEvent>, want: &str) -> Result<Value, String> {
    let ev = common::wait_event(rx, "message.done", |e| {
        common::frame_of(e)
            .is_some_and(|f| f.data["kind"] == "message.done" && f.data["status"] == want)
    })
    .await?;
    Ok(common::frame_of(&ev)
        .map(|f| f.data.clone())
        .unwrap_or_default())
}

#[tokio::test]
async fn chat_send_and_stream_roundtrip() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    assert!(c.subscribe("session.1").is_ok());
    let id = uuid();
    assert!(c.enqueue_message(SESSION, &id, "hello", &[]).is_ok());
    let created = common::wait_event(&rx, "message.created", |e| {
        common::frame_of(e).is_some_and(|f| {
            f.data["kind"] == "message.created" && f.data["message"]["role"] == "user"
        })
    })
    .await;
    assert!(created.is_ok(), "{created:?}");
    let done = wait_done(&rx, "done").await;
    assert!(done.is_ok(), "{done:?}");
    let h = http(&d, Some(d.admin_token()));
    let msgs = history(&h).await;
    let user = msgs.iter().find(|m| m["client_id"] == id.as_str());
    assert!(user.is_some_and(|m| m["body_md"] == "hello"), "{msgs:?}");
    let agent = msgs.iter().find(|m| m["role"] == "agent");
    assert!(
        agent
            .is_some_and(|m| m["status"] == "done"
                && m["body_md"].as_str().is_some_and(|b| b.contains("hello"))),
        "{msgs:?}"
    );
}

#[tokio::test]
async fn chat_stop_roundtrip() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    assert!(c.subscribe("session.1").is_ok());
    assert!(c.enqueue_message(SESSION, &uuid(), "/slow", &[]).is_ok());
    let first = common::wait_event(&rx, "first delta", |e| {
        common::frame_of(e).is_some_and(|f| f.data["kind"] == "message.delta")
    })
    .await;
    assert!(first.is_ok(), "{first:?}");
    let h = http(&d, Some(d.admin_token()));
    assert!(h.stop_session(SESSION).await.is_ok());
    assert!(wait_done(&rx, "stopped").await.is_ok());
    let msgs = history(&h).await;
    let agent = msgs.iter().find(|m| m["role"] == "agent");
    assert!(
        agent.is_some_and(
            |m| m["status"] == "stopped" && !m["body_md"].as_str().unwrap_or("").is_empty()
        ),
        "{msgs:?}"
    );
}

#[tokio::test]
async fn send_duplicate_client_id_idempotent() {
    let d = start(Config::temp().no_ticker()).await;
    let h = http(&d, Some(d.admin_token()));
    let id = uuid();
    let first = h.send_message(SESSION, &id, "one", &[]).await;
    assert!(matches!(first, Ok(SendOutcome::Created(_))), "{first:?}");
    let second = h.send_message(SESSION, &id, "one", &[]).await;
    assert!(
        matches!(second, Ok(SendOutcome::Duplicate(_))),
        "{second:?}"
    );
    let other = h.send_message(SESSION, &id, "other text", &[]).await;
    let code = other.err().and_then(|e| e.api().map(|a| a.status));
    assert_eq!(code, Some(409));
    let users = history(&h)
        .await
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    assert_eq!(users, 1);
}

#[tokio::test]
async fn chat_queue_no_duplicates() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    common::wait_state(&rx, core_client::ConnState::Connected)
        .await
        .expect("connection");
    d.drop_ws();
    common::wait_state(&rx, core_client::ConnState::Reconnecting)
        .await
        .expect("drop");
    let ids: Vec<String> = (0..3).map(|_| uuid()).collect();
    for (i, id) in ids.iter().enumerate() {
        assert!(
            c.enqueue_message(SESSION, id, &format!("queued {i}"), &[])
                .is_ok()
        );
    }
    d.restore();
    let mut delivered = 0;
    while delivered < 3 {
        let ev = common::wait_event(&rx, "delivery", |e| {
            matches!(e, CoreEvent::Delivered { .. })
        })
        .await;
        assert!(ev.is_ok(), "{ev:?}");
        delivered += 1;
    }
    let h = http(&d, Some(d.admin_token()));
    let users: Vec<Value> = history(&h)
        .await
        .into_iter()
        .filter(|m| m["role"] == "user")
        .collect();
    let got: Vec<&str> = users
        .iter()
        .filter_map(|m| m["client_id"].as_str())
        .collect();
    assert_eq!(
        got,
        ids.iter().map(String::as_str).collect::<Vec<_>>(),
        "exactly three, order preserved"
    );
}

#[tokio::test]
async fn chat_survives_ws_drop_mid_stream() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    assert!(c.subscribe("session.1").is_ok());
    assert!(
        c.enqueue_message(SESSION, &uuid(), "/drop cut", &[])
            .is_ok()
    );
    let _ = common::wait_event(&rx, "delta", |e| {
        common::frame_of(e).is_some_and(|f| f.data["kind"] == "message.delta")
    })
    .await;
    let lost = common::wait_state(&rx, core_client::ConnState::Reconnecting).await;
    assert!(lost.is_ok(), "{lost:?}");
    d.restore();
    let done = tokio::time::timeout(WAIT * 2, async { wait_done(&rx, "done").await }).await;
    assert!(matches!(done, Ok(Ok(_))), "{done:?}");
    let h = http(&d, Some(d.admin_token()));
    let msgs = history(&h).await;
    assert_eq!(msgs.iter().filter(|m| m["role"] == "user").count(), 1);
    assert_eq!(msgs.iter().filter(|m| m["role"] == "agent").count(), 1);
    let _ = json!(null);
}

#[tokio::test]
async fn queue_redelivery_reports_duplicate() {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    let id = uuid();
    for want in [false, true] {
        assert!(c.enqueue_message(SESSION, &id, "again", &[]).is_ok());
        let ev = common::wait_event(&rx, "delivery", |e| {
            matches!(e, CoreEvent::Delivered { .. })
        })
        .await;
        assert!(
            matches!(ev, Ok(CoreEvent::Delivered { duplicate, .. }) if duplicate == want),
            "{ev:?}"
        );
    }
    let h = http(&d, Some(d.admin_token()));
    assert_eq!(
        history(&h)
            .await
            .iter()
            .filter(|m| m["role"] == "user")
            .count(),
        1
    );
}
