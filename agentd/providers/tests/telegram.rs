//! Telegram Bot API against a local test server (`tg_*`): connect, send, outcomes, receive.

mod common;

use std::time::Duration;

use agentd_core::{ChannelAccount, ChannelProvider, ConnectKind, FlowStatus, Health, SendOutcome};
use agentd_providers::channels::Telegram;
use agentd_providers::{HttpClient, Timeouts};
use common::{Script, Seen, keys_with, serve};

const TOKEN: &str = "123456:SECRET-tg-token";

fn acc() -> ChannelAccount {
    ChannelAccount {
        id: 7,
        kind: "tg".into(),
    }
}

fn json(status: u16, body: &str) -> Script {
    Script {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        chunks: vec![(Duration::ZERO, body.as_bytes().to_vec())],
        hang: false,
        head_delay: Duration::ZERO,
    }
}

async fn tg(script: Script, with_token: bool) -> (Telegram, common::Server, tempfile::TempDir) {
    let server = serve(script).await;
    let refs: Vec<(&str, &str)> = if with_token {
        vec![("account.7", TOKEN)]
    } else {
        vec![]
    };
    let (dir, keys) = keys_with(&refs);
    let t = Telegram::new(HttpClient::new(Timeouts::default()), keys, &server.url);
    (t, server, dir)
}

fn last(s: &common::Server) -> Seen {
    s.seen
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("a request was made")
}

#[tokio::test]
async fn tg_connect_waits_for_token_then_checks_it() {
    let (t, srv, _d) = tg(json(200, r#"{"ok":true,"result":{"id":1}}"#), false).await;
    let f = t.connect(&acc()).await;
    assert_eq!(
        (f.kind, f.status),
        (ConnectKind::Token, FlowStatus::WaitingUser)
    );
    assert!(
        srv.seen.lock().unwrap().is_empty(),
        "no token: no network calls"
    );
    let (t, srv, _d) = tg(json(200, r#"{"ok":true,"result":{"id":1}}"#), true).await;
    assert_eq!(t.connect(&acc()).await.status, FlowStatus::Connected);
    assert!(
        last(&srv)
            .request_line
            .contains(&format!("/bot{TOKEN}/getMe"))
    );
    let (t, _s, _d) = tg(
        json(401, r#"{"ok":false,"description":"Unauthorized"}"#),
        true,
    )
    .await;
    assert_eq!(t.connect(&acc()).await.status, FlowStatus::Failed);
    let (t, _s, _d) = tg(json(200, r#"{"ok":false}"#), true).await;
    assert_eq!(
        t.connect(&acc()).await.status,
        FlowStatus::Failed,
        "200 without ok: not connected"
    );
}

#[tokio::test]
async fn tg_connect_network_failure_is_failed_and_health_down() {
    let (_d, keys) = keys_with(&[("account.7", TOKEN)]);
    let t = Telegram::new(
        HttpClient::new(Timeouts::default()),
        keys,
        "http://127.0.0.1:1",
    );
    assert_eq!(t.connect(&acc()).await.status, FlowStatus::Failed);
    assert_eq!(t.health(&acc()).await, Health::Down);
}

#[tokio::test]
async fn tg_health_levels() {
    let (t, _s, _d) = tg(json(200, r#"{"ok":true}"#), true).await;
    assert_eq!(t.health(&acc()).await, Health::Ok);
    let (t, _s, _d) = tg(json(502, "{}"), true).await;
    assert_eq!(t.health(&acc()).await, Health::Degraded);
    let (t, _s, _d) = tg(json(200, "{}"), false).await;
    assert_eq!(t.health(&acc()).await, Health::Down, "no token");
}

#[tokio::test]
async fn tg_send_delivered_carries_message_id() {
    let (t, srv, _d) = tg(json(200, r#"{"ok":true,"result":{"message_id":42}}"#), true).await;
    let out = t.send(&acc(), "@bob", "hello", "k1").await;
    assert_eq!(
        out,
        SendOutcome::Delivered {
            ext_id: Some("42".into())
        }
    );
    let seen = last(&srv);
    assert!(
        seen.request_line
            .contains(&format!("/bot{TOKEN}/sendMessage"))
    );
    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(
        (body["chat_id"].as_str(), body["text"].as_str()),
        (Some("@bob"), Some("hello"))
    );
}

#[tokio::test]
async fn tg_send_confirmed_rejections() {
    for (status, code) in [
        (400, "bad_request"),
        (401, "unauthorized"),
        (403, "forbidden"),
    ] {
        let (t, _s, _d) = tg(json(status, r#"{"ok":false}"#), true).await;
        assert_eq!(
            t.send(&acc(), "1", "x", "k").await,
            SendOutcome::Rejected(code),
            "{status}"
        );
    }
    let (t, srv, _d) = tg(json(200, "{}"), false).await;
    assert_eq!(
        t.send(&acc(), "1", "x", "k").await,
        SendOutcome::Rejected("no_token")
    );
    assert!(srv.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tg_send_ambiguous_results_are_unknown() {
    for (status, body) in [
        (429, "{}"),
        (500, "{}"),
        (502, "<html>"),
        (200, "not json"),
        (200, r#"{"ok":false}"#),
    ] {
        let (t, _s, _d) = tg(json(status, body), true).await;
        assert_eq!(
            t.send(&acc(), "1", "x", "k").await,
            SendOutcome::Unknown,
            "{status} {body}"
        );
    }
    let (_d, keys) = keys_with(&[("account.7", TOKEN)]);
    let t = Telegram::new(
        HttpClient::new(Timeouts::default()),
        keys,
        "http://127.0.0.1:1",
    );
    assert_eq!(
        t.send(&acc(), "1", "x", "k").await,
        SendOutcome::Unknown,
        "connection drop"
    );
}

#[tokio::test]
async fn tg_poll_parses_text_and_advances_offset() {
    let updates = r#"{"ok":true,"result":[
        {"update_id":10,"message":{"message_id":1,"chat":{"id":99},"from":{"username":"vasya"},"text":"hello"}},
        {"update_id":11,"message":{"message_id":2,"chat":{"id":98},"text":"no username"}},
        {"update_id":12,"message":{"message_id":3,"chat":{"id":97},"from":{"username":"x"},"photo":[]}}]}"#;
    let (t, srv, _d) = tg(json(200, updates), true).await;
    let got = t.poll(&acc()).await;
    let pairs: Vec<(&str, &str, &str)> = got
        .iter()
        .map(|m| (m.ext_id.as_str(), m.handle.as_str(), m.body.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [("10", "@vasya", "hello"), ("11", "98", "no username")],
        "text only"
    );
    let first: serde_json::Value = serde_json::from_str(&last(&srv).body).unwrap();
    assert_eq!(first["offset"], 0);
    t.poll(&acc()).await;
    let second: serde_json::Value = serde_json::from_str(&last(&srv).body).unwrap();
    assert_eq!(second["offset"], 13, "all updates up to 12 acknowledged");
}

#[tokio::test]
async fn tg_poll_failure_returns_nothing() {
    let (t, _s, _d) = tg(json(500, "{}"), true).await;
    assert!(t.poll(&acc()).await.is_empty());
    let (t, _s, _d) = tg(json(200, "{}"), false).await;
    assert!(t.poll(&acc()).await.is_empty());
}

#[tokio::test]
async fn tg_capabilities() {
    let (t, _s, _d) = tg(json(200, "{}"), false).await;
    assert_eq!(t.kind(), "tg");
    assert_eq!(t.capabilities().len(), 2);
}
