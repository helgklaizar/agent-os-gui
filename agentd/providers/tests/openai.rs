//! OpenAI-compatible and local provider.

mod common;

use std::sync::Arc;
use std::time::Duration;

use agentd_core::{ModelProvider, ProviderEvent, ProviderKind, SystemClock, Usage};
use agentd_providers::http::{HttpClient, Timeouts};
use agentd_providers::openai::{OpenAiCompat, OpenAiParser};
use agentd_providers::sse::SseParser;
use common::{Script, account, collect, keys_with, request, serve, text_of};

const KEY: &str = "sk-openai-test-KEY-456";

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/openai_stream.sse"
    ))
    .unwrap()
}

fn provider(keys: Arc<agentd_providers::KeysFile>, local: bool) -> OpenAiCompat {
    let t = Timeouts {
        connect: Duration::from_millis(500),
        first_event: Duration::from_millis(500),
        idle: Duration::from_millis(500),
    };
    OpenAiCompat::new(HttpClient::new(t), keys, Arc::new(SystemClock), local)
}

#[test]
fn openai_sse_parse_fixture() {
    let mut sse = SseParser::new();
    let mut parser = OpenAiParser::new();
    let mut out = Vec::new();
    let mut done = false;
    for ev in sse.feed(fixture().as_bytes()) {
        done = parser.on_event(&ev, &mut out);
    }
    assert!(done, "[DONE] ends the stream");
    assert_eq!(text_of(&out), "Hello, world");
    assert!(
        out.contains(&ProviderEvent::Usage(Usage {
            tokens_in: 11,
            tokens_out: 7
        })),
        "{out:?}"
    );
}

#[tokio::test]
async fn openai_request_shape_and_bearer_from_store() {
    let srv = serve(Script::sse(&fixture())).await;
    let (_d, keys) = keys_with(&[("account.1", KEY)]);
    let ev = collect(
        &provider(keys, false),
        request(account(
            ProviderKind::OpenAiCompat,
            &format!("{}/v1", srv.url),
            Some("account.1"),
        )),
    )
    .await;
    assert_eq!(text_of(&ev), "Hello, world");
    let seen = srv.seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("POST /v1/chat/completions "),
        "{}",
        seen[0].request_line
    );
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(
        body["messages"][0],
        serde_json::json!({"role":"system","content":"system text"})
    );
    assert_eq!(
        body["messages"][1],
        serde_json::json!({"role":"user","content":"question"})
    );
    assert!(!seen[0].body.contains(KEY));
}

#[tokio::test]
async fn openai_missing_key_is_no_key() {
    let srv = serve(Script::sse(&fixture())).await;
    let (_d, keys) = keys_with(&[]);
    let ev = collect(
        &provider(keys, false),
        request(account(
            ProviderKind::OpenAiCompat,
            &srv.url,
            Some("account.1"),
        )),
    )
    .await;
    let [ProviderEvent::Error(e)] = ev.as_slice() else {
        panic!("{ev:?}")
    };
    assert_eq!(e.code, "no_key");
    assert!(srv.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn local_provider_no_limits() {
    let srv = serve(Script::sse(&fixture())).await;
    let (_d, keys) = keys_with(&[]);
    let ev = collect(
        &provider(keys, true),
        request(account(ProviderKind::Local, &srv.url, None)),
    )
    .await;
    assert_eq!(text_of(&ev), "Hello, world", "{ev:?}");
    assert!(
        !ev.iter().any(|e| matches!(e, ProviderEvent::RateLimit(_))),
        "no limits"
    );
    let seen = srv.seen.lock().unwrap();
    assert!(
        seen[0].header("authorization").is_none(),
        "no key, no header"
    );
}

#[tokio::test]
async fn openai_rate_limit_headers_become_event() {
    let s = Script::sse(&fixture())
        .header("x-ratelimit-limit-requests", "60")
        .header("x-ratelimit-remaining-requests", "6")
        .header("x-ratelimit-reset-requests", "1s");
    let srv = serve(s).await;
    let (_d, keys) = keys_with(&[("account.1", KEY)]);
    let ev = collect(
        &provider(keys, false),
        request(account(
            ProviderKind::OpenAiCompat,
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
    assert_eq!(
        (rl.requests.unwrap().limit, rl.requests.unwrap().remaining),
        (60, 6)
    );
}

#[tokio::test]
async fn openai_http_errors_map() {
    for (st, code) in [
        (401, "unauthorized"),
        (429, "rate_limited"),
        (503, "upstream"),
    ] {
        let srv = serve(Script::status(st)).await;
        let (_d, keys) = keys_with(&[("account.1", KEY)]);
        let ev = collect(
            &provider(keys, false),
            request(account(
                ProviderKind::OpenAiCompat,
                &srv.url,
                Some("account.1"),
            )),
        )
        .await;
        let Some(ProviderEvent::Error(e)) = ev.last() else {
            panic!("{ev:?}")
        };
        assert_eq!(e.code, code, "{st}");
    }
}

#[tokio::test]
async fn openai_check_models_endpoint_local_without_key() {
    let srv = serve(Script::status(200)).await;
    let (_d, keys) = keys_with(&[]);
    let acc = account(ProviderKind::Local, &srv.url, None);
    provider(keys, true).check(&acc).await.unwrap();
    assert!(
        srv.seen.lock().unwrap()[0]
            .request_line
            .starts_with("GET /models")
    );
}
