//! Whapi (WhatsApp) against a local test server (`whapi_*`). The token comes from the secret store; there is no live channel.

mod common;

use std::time::Duration;

use agentd_core::{ChannelAccount, ChannelProvider, ConnectKind, FlowStatus, Health, SendOutcome};
use agentd_providers::channels::Whapi;
use agentd_providers::{HttpClient, Timeouts};
use common::{Script, Seen, keys_with, serve};

const TOKEN: &str = "whapi-SECRET-token";

fn acc() -> ChannelAccount {
    ChannelAccount {
        id: 4,
        kind: "wa".into(),
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

async fn wa(script: Script, token: bool) -> (Whapi, common::Server, tempfile::TempDir) {
    let srv = serve(script).await;
    let refs: Vec<(&str, &str)> = if token {
        vec![("account.4", TOKEN)]
    } else {
        vec![]
    };
    let (dir, keys) = keys_with(&refs);
    (
        Whapi::new(HttpClient::new(Timeouts::default()), keys, &srv.url),
        srv,
        dir,
    )
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
async fn whapi_connect_states_from_health() {
    let (w, srv, _d) = wa(json(200, r#"{"status":{"code":4,"text":"AUTH"}}"#), true).await;
    let f = w.connect(&acc()).await;
    assert_eq!((f.kind, f.status), (ConnectKind::Qr, FlowStatus::Connected));
    let seen = last(&srv);
    assert!(
        seen.request_line.starts_with("GET /health"),
        "{}",
        seen.request_line
    );
    assert_eq!(
        seen.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    let (w, _s, _d) = wa(json(200, r#"{"status":{"code":2,"text":"QR"}}"#), true).await;
    let f = w.connect(&acc()).await;
    assert_eq!(
        f.status,
        FlowStatus::WaitingUser,
        "number unlinked: scan the QR code in the Whapi dashboard"
    );
    let (w, _s, _d) = wa(json(401, "{}"), true).await;
    assert_eq!(w.connect(&acc()).await.status, FlowStatus::Failed);
    let (w, srv, _d) = wa(json(200, "{}"), false).await;
    assert_eq!(w.connect(&acc()).await.status, FlowStatus::WaitingUser);
    assert!(
        srv.seen.lock().unwrap().is_empty(),
        "no token: no network calls"
    );
}

#[tokio::test]
async fn whapi_health_levels_qr_is_down() {
    let (w, _s, _d) = wa(json(200, r#"{"status":{"text":"AUTH"}}"#), true).await;
    assert_eq!(w.health(&acc()).await, Health::Ok);
    let (w, _s, _d) = wa(json(200, r#"{"status":{"text":"QR"}}"#), true).await;
    assert_eq!(w.health(&acc()).await, Health::Down);
    let (w, _s, _d) = wa(json(503, "{}"), true).await;
    assert_eq!(w.health(&acc()).await, Health::Degraded);
    let (w, _s, _d) = wa(json(200, "{}"), false).await;
    assert_eq!(w.health(&acc()).await, Health::Down);
}

#[tokio::test]
async fn whapi_send_delivered_and_body() {
    let (w, srv, _d) = wa(
        json(200, r#"{"sent":true,"message":{"id":"abc123"}}"#),
        true,
    )
    .await;
    let out = w.send(&acc(), "79990001122", "hello", "k").await;
    assert_eq!(
        out,
        SendOutcome::Delivered {
            ext_id: Some("abc123".into())
        }
    );
    let seen = last(&srv);
    assert!(seen.request_line.starts_with("POST /messages/text"));
    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(
        (body["to"].as_str(), body["body"].as_str()),
        (Some("79990001122"), Some("hello"))
    );
    assert_eq!(
        seen.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn whapi_send_outcomes() {
    for (st, code) in [
        (400, "bad_request"),
        (401, "unauthorized"),
        (403, "forbidden"),
    ] {
        let (w, _s, _d) = wa(json(st, "{}"), true).await;
        assert_eq!(
            w.send(&acc(), "1", "x", "k").await,
            SendOutcome::Rejected(code),
            "{st}"
        );
    }
    for (st, body) in [
        (429, "{}"),
        (500, "{}"),
        (200, "not json"),
        (200, r#"{"sent":false}"#),
    ] {
        let (w, _s, _d) = wa(json(st, body), true).await;
        assert_eq!(
            w.send(&acc(), "1", "x", "k").await,
            SendOutcome::Unknown,
            "{st} {body}"
        );
    }
    let (w, srv, _d) = wa(json(200, "{}"), false).await;
    assert_eq!(
        w.send(&acc(), "1", "x", "k").await,
        SendOutcome::Rejected("no_token")
    );
    assert!(srv.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn whapi_poll_takes_incoming_text_only() {
    let body = r#"{"messages":[
        {"id":"m1","from_me":false,"from":"79990001122","chat_id":"79990001122@s.whatsapp.net","type":"text","text":{"body":"hello"},"timestamp":100},
        {"id":"m2","from_me":true,"from":"me","type":"text","text":{"body":"mine"},"timestamp":101},
        {"id":"m3","from_me":false,"from":"79990003344","type":"image","timestamp":102}]}"#;
    let (w, srv, _d) = wa(json(200, body), true).await;
    let got = w.poll(&acc()).await;
    let pairs: Vec<(&str, &str, &str)> = got
        .iter()
        .map(|m| (m.ext_id.as_str(), m.handle.as_str(), m.body.as_str()))
        .collect();
    assert_eq!(pairs, [("m1", "79990001122", "hello")]);
    assert!(last(&srv).request_line.starts_with("GET /messages/list"));
    w.poll(&acc()).await;
    assert!(
        last(&srv).request_line.contains("time_from=103"),
        "{}",
        last(&srv).request_line
    );
}

#[tokio::test]
async fn whapi_poll_failure_and_capabilities() {
    let (w, _s, _d) = wa(json(500, "{}"), true).await;
    assert!(w.poll(&acc()).await.is_empty());
    let (w, _s, _d) = wa(json(200, "{}"), false).await;
    assert!(w.poll(&acc()).await.is_empty());
    assert_eq!(w.kind(), "wa");
    assert_eq!(w.capabilities().len(), 2);
}
