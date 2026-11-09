//! Anthropic provider: stream parsing, request, errors, limits.

mod common;

use std::sync::Arc;
use std::time::Duration;

use agentd_core::{ModelProvider, ProviderEvent, ProviderKind, SystemClock, Usage};
use agentd_providers::anthropic::{Anthropic, AnthropicParser};
use agentd_providers::http::{HttpClient, Timeouts};
use agentd_providers::sse::SseParser;
use common::{Script, account, collect, keys_with, request, serve, text_of};

const KEY: &str = "sk-ant-test-KEY-123";

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/anthropic_stream.sse"
    ))
    .unwrap()
}

fn provider(keys: Arc<agentd_providers::KeysFile>) -> Anthropic {
    let t = Timeouts {
        connect: Duration::from_millis(500),
        first_event: Duration::from_millis(500),
        idle: Duration::from_millis(500),
    };
    Anthropic::new(HttpClient::new(t), keys, Arc::new(SystemClock))
}

#[test]
fn anthropic_sse_parse_fixture() {
    let mut sse = SseParser::new();
    let mut parser = AnthropicParser::new();
    let mut out = Vec::new();
    let mut done = false;
    for ev in sse.feed(fixture().as_bytes()) {
        done = parser.on_event(&ev, &mut out);
    }
    assert!(done, "message_stop ends the stream");
    assert_eq!(text_of(&out), "Hello, world");
    assert!(
        out.contains(&ProviderEvent::Usage(Usage {
            tokens_in: 25,
            tokens_out: 15
        })),
        "{out:?}"
    );
}

#[test]
fn anthropic_error_event_maps_to_normalized() {
    for (kind, code, retry) in [
        ("overloaded_error", "overloaded", true),
        ("rate_limit_error", "rate_limited", true),
        ("authentication_error", "unauthorized", false),
        ("api_error", "upstream", true),
        ("invalid_request_error", "bad_response", false),
    ] {
        let data = format!(r#"{{"type":"error","error":{{"type":"{kind}","message":"details"}}}}"#);
        let mut out = Vec::new();
        let done = AnthropicParser::new().on_event(
            &agentd_providers::sse::SseEvent {
                event: Some("error".into()),
                data,
            },
            &mut out,
        );
        assert!(done);
        let Some(ProviderEvent::Error(e)) = out.last() else {
            panic!("{out:?}")
        };
        assert_eq!((e.code, e.retryable), (code, retry), "{kind}");
        assert!(!format!("{e:?}").contains("details"));
    }
}

#[tokio::test]
async fn anthropic_request_shape_and_key_from_store() {
    let srv = serve(Script::sse(&fixture())).await;
    let (_d, keys) = keys_with(&[("account.1", KEY)]);
    let ev = collect(
        &provider(keys),
        request(account(
            ProviderKind::Anthropic,
            &srv.url,
            Some("account.1"),
        )),
    )
    .await;
    assert_eq!(text_of(&ev), "Hello, world");
    let seen = srv.seen.lock().unwrap();
    assert!(
        seen[0].request_line.starts_with("POST /v1/messages "),
        "{}",
        seen[0].request_line
    );
    assert_eq!(seen[0].header("x-api-key"), Some(KEY));
    assert!(seen[0].header("anthropic-version").is_some());
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["system"], "system text");
    assert_eq!(
        body["messages"][0],
        serde_json::json!({"role":"user","content":"question"})
    );
    assert!(!seen[0].body.contains(KEY), "key is not in the body");
}

#[tokio::test]
async fn anthropic_missing_key_is_no_key_without_network() {
    let srv = serve(Script::sse(&fixture())).await;
    let (_d, keys) = keys_with(&[]);
    let ev = collect(
        &provider(keys),
        request(account(
            ProviderKind::Anthropic,
            &srv.url,
            Some("account.1"),
        )),
    )
    .await;
    let [ProviderEvent::Error(e)] = ev.as_slice() else {
        panic!("{ev:?}")
    };
    assert_eq!((e.code, e.retryable), ("no_key", false));
    assert!(srv.seen.lock().unwrap().is_empty(), "no outbound request");
}

#[tokio::test]
async fn anthropic_http_errors_map() {
    for (st, code, retry) in [
        (401, "unauthorized", false),
        (429, "rate_limited", true),
        (500, "upstream", true),
        (529, "overloaded", true),
    ] {
        let srv = serve(Script::status(st)).await;
        let (_d, keys) = keys_with(&[("account.1", KEY)]);
        let ev = collect(
            &provider(keys),
            request(account(
                ProviderKind::Anthropic,
                &srv.url,
                Some("account.1"),
            )),
        )
        .await;
        let Some(ProviderEvent::Error(e)) = ev.last() else {
            panic!("{ev:?}")
        };
        assert_eq!((e.code, e.retryable), (code, retry), "{st}");
    }
}

#[tokio::test]
async fn anthropic_rate_limit_headers_become_event() {
    let s = Script::sse(&fixture())
        .header("anthropic-ratelimit-requests-limit", "50")
        .header("anthropic-ratelimit-requests-remaining", "10")
        .header("anthropic-ratelimit-requests-reset", "2026-11-03T10:00:00Z");
    let srv = serve(s).await;
    let (_d, keys) = keys_with(&[("account.1", KEY)]);
    let ev = collect(
        &provider(keys),
        request(account(
            ProviderKind::Anthropic,
            &srv.url,
            Some("account.1"),
        )),
    )
    .await;
    let rl = ev
        .iter()
        .find_map(|e| match e {
            ProviderEvent::RateLimit(r) => Some(*r),
            _ => None,
        })
        .expect("RateLimit");
    assert_eq!(rl.requests.unwrap().limit, 50);
    assert_eq!(rl.requests.unwrap().remaining, 10);
}

#[tokio::test]
async fn anthropic_check_uses_models_endpoint() {
    let srv = serve(Script::status(200)).await;
    let (_d, keys) = keys_with(&[("account.1", KEY)]);
    let acc = account(ProviderKind::Anthropic, &srv.url, Some("account.1"));
    provider(keys.clone()).check(&acc).await.unwrap();
    assert!(
        srv.seen.lock().unwrap()[0]
            .request_line
            .starts_with("GET /v1/models")
    );
    let bad = serve(Script::status(401)).await;
    let acc = account(ProviderKind::Anthropic, &bad.url, Some("account.1"));
    assert_eq!(
        provider(keys).check(&acc).await.unwrap_err().code,
        "unauthorized"
    );
}
