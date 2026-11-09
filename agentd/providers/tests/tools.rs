//! The tool catalog is sent to the provider; tool calls are parsed from the stream.

mod common;

use std::sync::Arc;
use std::time::Duration;

use agentd_core::{ProviderEvent, ProviderKind, SystemClock, ToolCall, ToolSpec};
use agentd_providers::anthropic::{Anthropic, AnthropicParser};
use agentd_providers::http::{HttpClient, Timeouts};
use agentd_providers::openai::{OpenAiCompat, OpenAiParser};
use agentd_providers::sse::SseParser;
use common::{Script, account, collect, keys_with, request, serve};
use serde_json::{Value, json};

const ANTHROPIC_TOOL_STREAM: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Reading.\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"mcp__fs__read\",\"input\":{}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pa\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"th\\\":\\\"/a\\\"}\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

const OPENAI_TOOL_STREAM: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Running.\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"command\\\":\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"ls\\\"}\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"browse\",\"arguments\":\"{\\\"url\\\":\\\"https://a.test/\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n\n",
    "data: [DONE]\n\n",
);

fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "mcp__fs__read".into(),
        description: "Reads a file".into(),
        schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    }]
}

fn timeouts() -> Timeouts {
    Timeouts {
        connect: Duration::from_millis(500),
        first_event: Duration::from_millis(500),
        idle: Duration::from_millis(500),
    }
}

fn calls_of(ev: &[ProviderEvent]) -> Vec<ToolCall> {
    ev.iter()
        .filter_map(|e| match e {
            ProviderEvent::ToolCall(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn anthropic_tool_use_blocks_become_tool_calls() {
    let mut sse = SseParser::new();
    let mut parser = AnthropicParser::new();
    let mut out = Vec::new();
    for ev in sse.feed(ANTHROPIC_TOOL_STREAM.as_bytes()) {
        parser.on_event(&ev, &mut out);
    }
    assert_eq!(
        calls_of(&out),
        vec![ToolCall {
            id: "toolu_1".into(),
            name: "mcp__fs__read".into(),
            args: json!({"path": "/a"}),
        }],
        "{out:?}"
    );
}

#[test]
fn anthropic_broken_tool_json_is_not_a_call() {
    let broken = ANTHROPIC_TOOL_STREAM.replace("th\\\":\\\"/a\\\"}\"", "th\\\":");
    let mut sse = SseParser::new();
    let mut parser = AnthropicParser::new();
    let mut out = Vec::new();
    for ev in sse.feed(broken.as_bytes()) {
        parser.on_event(&ev, &mut out);
    }
    assert!(
        calls_of(&out).is_empty(),
        "truncated arguments are not a call: {out:?}"
    );
}

#[test]
fn openai_tool_calls_are_assembled_by_index() {
    let mut sse = SseParser::new();
    let mut parser = OpenAiParser::new();
    let mut out = Vec::new();
    for ev in sse.feed(OPENAI_TOOL_STREAM.as_bytes()) {
        parser.on_event(&ev, &mut out);
    }
    assert_eq!(
        calls_of(&out),
        vec![
            ToolCall {
                id: "call_1".into(),
                name: "shell".into(),
                args: json!({"command": "ls"}),
            },
            ToolCall {
                id: "call_2".into(),
                name: "browse".into(),
                args: json!({"url": "https://a.test/"}),
            },
        ],
        "{out:?}"
    );
}

#[tokio::test]
async fn anthropic_request_carries_tools_and_returns_calls() {
    let srv = serve(Script::sse(ANTHROPIC_TOOL_STREAM)).await;
    let (_d, keys) = keys_with(&[("account.1", "sk-ant-k")]);
    let p = Anthropic::new(HttpClient::new(timeouts()), keys, Arc::new(SystemClock));
    let mut req = request(account(
        ProviderKind::Anthropic,
        &srv.url,
        Some("account.1"),
    ));
    req.tools = tools();
    let ev = collect(&p, req).await;
    assert_eq!(calls_of(&ev).len(), 1, "{ev:?}");
    let seen = srv.seen.lock().unwrap();
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["tools"][0]["name"], "mcp__fs__read", "{body}");
    assert_eq!(body["tools"][0]["description"], "Reads a file");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
}

#[tokio::test]
async fn openai_request_carries_tools_and_returns_calls() {
    let srv = serve(Script::sse(OPENAI_TOOL_STREAM)).await;
    let (_d, keys) = keys_with(&[("account.1", "sk-oa-k")]);
    let p = OpenAiCompat::new(
        HttpClient::new(timeouts()),
        keys,
        Arc::new(SystemClock),
        false,
    );
    let mut req = request(account(
        ProviderKind::OpenAiCompat,
        &format!("{}/v1", srv.url),
        Some("account.1"),
    ));
    req.tools = tools();
    let ev = collect(&p, req).await;
    assert_eq!(calls_of(&ev).len(), 2, "{ev:?}");
    let seen = srv.seen.lock().unwrap();
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["tools"][0]["type"], "function", "{body}");
    assert_eq!(body["tools"][0]["function"]["name"], "mcp__fs__read");
    assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
}

#[tokio::test]
async fn empty_catalog_sends_no_tools_field() {
    let srv = serve(Script::sse(ANTHROPIC_TOOL_STREAM)).await;
    let (_d, keys) = keys_with(&[("account.1", "sk-ant-k")]);
    let p = Anthropic::new(HttpClient::new(timeouts()), keys, Arc::new(SystemClock));
    let req = request(account(
        ProviderKind::Anthropic,
        &srv.url,
        Some("account.1"),
    ));
    collect(&p, req).await;
    let seen = srv.seen.lock().unwrap();
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert!(body.get("tools").is_none(), "{body}");
}
