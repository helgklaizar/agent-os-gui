//! Contract parsing without a network: error codes, WS frames, queue, reconnect pause, address from the environment.

use std::path::PathBuf;
use std::time::Duration;

use core_client::api::{
    ApiError, ClientOp, ErrorCode, FrameKind, ServerMessage, TokenInfo, TokenIssued,
    parse_server_message,
};
use core_client::queue::{OutboundQueue, Outgoing, Target};
use core_client::{Endpoint, ReconnectPolicy, Token};
use serde_json::json;

/// api.md section 3 table: HTTP status, `code`, whether to retry.
const TABLE: &[(u16, &str, bool)] = &[
    (400, "bad_request", false),
    (401, "unauthorized", false),
    (403, "permission_denied", false),
    (403, "insufficient_scope", false),
    (403, "forbidden_method", false),
    (403, "forbidden_origin", false),
    (404, "not_found", false),
    (409, "conflict", false),
    (409, "client_id_conflict", false),
    (412, "precondition_failed", false),
    (413, "payload_too_large", false),
    (422, "validation_failed", false),
    (428, "precondition_required", false),
    (429, "rate_limited", true),
    (500, "internal", true),
    (503, "core_down", true),
    (503, "migrating", true),
];

fn body(code: &str, extra: serde_json::Value) -> Vec<u8> {
    let mut e = json!({ "code": code, "message": "text", "details": {}, "request_id": "0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a" });
    if let (Some(m), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        m.extend(x.clone());
    }
    serde_json::to_vec(&json!({ "error": e })).unwrap_or_default()
}

#[test]
fn error_code_table_roundtrip() {
    for (status, code, _) in TABLE {
        let parsed = ErrorCode::parse(code);
        assert!(
            !matches!(parsed, ErrorCode::Unknown(_)),
            "code {code} not recognised"
        );
        assert_eq!(parsed.as_str(), *code);
        let _ = status;
    }
    assert_eq!(
        ErrorCode::parse("new_code_from_future"),
        ErrorCode::Unknown("new_code_from_future".into())
    );
    assert_eq!(ErrorCode::Unknown("x".into()).as_str(), "x");
}

#[test]
fn api_error_parses_every_table_row() {
    for (status, code, retryable) in TABLE {
        let err = ApiError::from_response(*status, &body(code, json!({})), None, None);
        assert_eq!(err.status, *status);
        assert_eq!(err.code.as_str(), *code, "status {status}");
        assert_eq!(err.retryable, *retryable, "retry for {code}");
        assert_eq!(err.message, "text");
        assert_eq!(
            err.request_id.as_deref(),
            Some("0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a")
        );
    }
}

#[test]
fn api_error_body_flag_wins_and_retry_after_parsed() {
    let err = ApiError::from_response(
        429,
        &body("rate_limited", json!({ "retryable": false })),
        Some("hdr-id"),
        Some("7"),
    );
    assert!(!err.retryable);
    assert_eq!(err.retry_after, Some(Duration::from_secs(7)));
    let err = ApiError::from_response(
        401,
        &body(
            "unauthorized",
            json!({ "details": { "reason": "token_in_url" } }),
        ),
        None,
        None,
    );
    assert_eq!(err.reason(), Some("token_in_url"));
}

#[test]
fn api_error_falls_back_to_status_for_garbage_body() {
    for (status, code) in [
        (401u16, "unauthorized"),
        (403, "insufficient_scope"),
        (404, "not_found"),
        (412, "precondition_failed"),
        (428, "precondition_required"),
        (429, "rate_limited"),
        (503, "core_down"),
        (500, "internal"),
    ] {
        let err = ApiError::from_response(status, b"<html>oops</html>", Some("req-1"), None);
        assert_eq!(err.code.as_str(), code, "status {status}");
        assert_eq!(err.request_id.as_deref(), Some("req-1"));
        assert!(
            !err.message.contains("oops"),
            "a foreign response body does not leak into the error text"
        );
    }
}

#[test]
fn server_message_parsing() {
    let snap = parse_server_message(
        r#"{"topic":"bar.sys","type":"snapshot","seq":1,"ts":1793571261,"data":{"cpu":14}}"#,
    );
    let Some(ServerMessage::Frame(f)) = snap else {
        panic!("snapshot not parsed: {snap:?}")
    };
    assert_eq!(
        (f.topic.as_str(), f.kind, f.seq, f.ts),
        ("bar.sys", FrameKind::Snapshot, 1, 1793571261)
    );
    assert_eq!(f.data, json!({ "cpu": 14 }));
    let delta = parse_server_message(
        r#"{"topic":"events","type":"delta","seq":2,"ts":1,"data":{"read":[1]},"extra":true}"#,
    );
    assert!(
        matches!(delta, Some(ServerMessage::Frame(f)) if f.kind == FrameKind::Delta && f.seq == 2)
    );
    let ev = parse_server_message(r#"{"topic":"events","type":"event","seq":3,"ts":1,"data":{}}"#);
    assert!(matches!(ev, Some(ServerMessage::Frame(f)) if f.kind == FrameKind::Event));
    assert_eq!(
        parse_server_message(r#"{"type":"pong"}"#),
        Some(ServerMessage::Pong)
    );
}

#[test]
fn server_message_unknown_and_garbage_ignored() {
    for text in [
        r#"{"topic":"x","type":"from_future","seq":1,"ts":1,"data":{}}"#,
        "not json",
        "[]",
        r#"{"type":"snapshot"}"#,
        "",
    ] {
        assert_eq!(parse_server_message(text), None, "{text}");
    }
}

#[test]
fn client_ops_serialize_to_contract() {
    let v =
        |op: ClientOp| serde_json::from_str::<serde_json::Value>(&op.to_json()).unwrap_or_default();
    assert_eq!(
        v(ClientOp::Subscribe("session.42".into())),
        json!({ "op": "subscribe", "topic": "session.42" })
    );
    assert_eq!(
        v(ClientOp::Unsubscribe("bar.sys".into())),
        json!({ "op": "unsubscribe", "topic": "bar.sys" })
    );
    assert_eq!(v(ClientOp::Ping), json!({ "op": "ping" }));
}

#[test]
fn reconnect_series_1_2_4_8_16_30_cap() {
    let p = ReconnectPolicy::default();
    let secs: Vec<u64> = (0..9).map(|a| p.delay(a, 0.0).as_secs()).collect();
    assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30, 30]);
    assert_eq!(
        p.delay(1000, 0.0),
        Duration::from_secs(30),
        "a large attempt number neither overflows nor exceeds the ceiling"
    );
}

#[test]
fn reconnect_jitter_only_lowers_and_stays_in_bounds() {
    let p = ReconnectPolicy::default();
    for attempt in 0..8 {
        let full = p.delay(attempt, 0.0);
        let low = p.delay(attempt, 0.999);
        assert!(
            low < full,
            "jitter must shorten the pause (attempt {attempt})"
        );
        assert!(
            low.as_secs_f64() >= full.as_secs_f64() * 0.79,
            "jitter is at most 20% (attempt {attempt})"
        );
        assert!(full <= Duration::from_secs(30));
    }
}

#[test]
fn endpoint_from_env_values() {
    let unix = |p: &str| Some(Endpoint::Unix(PathBuf::from(p)));
    assert_eq!(
        Endpoint::from_env_values(
            Some("/tmp/a.sock"),
            Some("tcp:127.0.0.1:9"),
            Some("/run/user/1")
        ),
        unix("/tmp/a.sock"),
        "AGENTD_SOCKET takes precedence"
    );
    assert_eq!(
        Endpoint::from_env_values(None, Some("tcp:127.0.0.1:4096"), None),
        Some(Endpoint::Tcp("127.0.0.1:4096".into()))
    );
    assert_eq!(
        Endpoint::from_env_values(None, None, Some("/run/user/1000")),
        unix("/run/user/1000/agentd.sock")
    );
    assert_eq!(
        Endpoint::from_env_values(None, None, None),
        None,
        "no address source means no fallback"
    );
    assert_eq!(
        Endpoint::from_env_values(None, Some("http://x"), None),
        None,
        "foreign AGENTD_ADDR format rejected"
    );
    assert_eq!(
        Endpoint::from_env_values(Some(""), None, None),
        None,
        "empty AGENTD_SOCKET is not accepted"
    );
}

#[test]
fn endpoint_host_header() {
    assert_eq!(
        Endpoint::Unix(PathBuf::from("/x")).host_header(),
        "localhost"
    );
    assert_eq!(
        Endpoint::Tcp("127.0.0.1:4096".into()).host_header(),
        "127.0.0.1:4096"
    );
}

#[test]
fn queue_keeps_order_and_rejects_duplicate_ids() {
    let o = |id: &str| Outgoing {
        client_id: id.into(),
        text: format!("t-{id}"),
        target: Target::Echo,
    };
    let mut q = OutboundQueue::new();
    assert!(q.is_empty());
    assert!(q.push(o("a")));
    assert!(q.push(o("b")));
    assert!(
        !q.push(o("a")),
        "the same client_id is not queued a second time"
    );
    assert_eq!(q.len(), 2);
    assert_eq!(q.front().map(|x| x.client_id.as_str()), Some("a"));
    assert_eq!(q.pop_front().map(|x| x.client_id), Some("a".into()));
    assert_eq!(q.pop_front().map(|x| x.client_id), Some("b".into()));
    assert_eq!(q.pop_front(), None);
    assert!(q.push(o("a")), "client_id is free again after sending");
}

#[test]
fn token_and_issued_debug_never_print_value() {
    let raw = "agt_secret-looking-value-for-test-0123456789abc";
    assert!(!format!("{:?}", Token::new(raw)).contains(raw));
    let issued: TokenIssued = serde_json::from_value(json!({
        "id": 1, "name": "n", "kind": "external", "scope": "chat", "revoked": false, "created_at": 1, "token": raw
    }))
    .expect("TokenIssued parses");
    assert_eq!(issued.token, raw);
    assert!(!format!("{issued:?}").contains(raw));
    let info: TokenInfo = serde_json::from_value(json!({ "id": 2, "name": "n", "kind": "module", "scope": "read", "revoked": true, "allowlist": ["GET /x"], "unknown_field": 1 })).expect("unknown fields are ignored");
    assert_eq!(info.allowlist, Some(vec!["GET /x".to_string()]));
}
