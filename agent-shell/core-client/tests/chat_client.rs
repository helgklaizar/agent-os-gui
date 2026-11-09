//! Pure parts of the chat client: send-response parsing, multipart, message queue.

use core_client::chat::SendOutcome;
use core_client::http::{RawResponse, Request};
use core_client::queue::{OutboundQueue, Outgoing, Target};

fn resp(status: u16, body: &str) -> RawResponse {
    RawResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn send_outcome_by_status() {
    let created = SendOutcome::from_response(&resp(201, r#"{"id":1}"#));
    assert!(matches!(created, Some(SendOutcome::Created(v)) if v["id"] == 1));
    let dup = SendOutcome::from_response(&resp(200, r#"{"id":1}"#));
    assert!(matches!(dup, Some(SendOutcome::Duplicate(v)) if v["id"] == 1));
    let ask = SendOutcome::from_response(&resp(202, r#"{"id":7,"status":"pending"}"#));
    assert!(matches!(ask, Some(SendOutcome::NeedsDecision(v)) if v["id"] == 7));
}

#[test]
fn send_outcome_rejects_unknown_status_and_garbage() {
    assert!(SendOutcome::from_response(&resp(204, "")).is_none());
    assert!(SendOutcome::from_response(&resp(201, "not json")).is_none());
}

#[test]
fn multipart_has_boundary_field_and_bytes() {
    let r = Request::multipart(
        "/v1/sessions/1/attachments",
        "a.txt",
        "text/plain",
        b"HELLO",
    );
    let (ctype, body) = r.raw.expect("raw body set");
    let boundary = ctype
        .strip_prefix("multipart/form-data; boundary=")
        .expect("type with boundary");
    assert!(boundary.len() >= 16);
    let text = String::from_utf8_lossy(&body).to_string();
    assert!(text.starts_with(&format!("--{boundary}\r\n")));
    assert!(text.contains(r#"name="file"; filename="a.txt""#));
    assert!(text.contains("Content-Type: text/plain\r\n\r\nHELLO\r\n"));
    assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    assert!(r.body.is_none());
}

#[test]
fn multipart_filename_cannot_break_headers() {
    let r = Request::multipart("/p", "x\"\r\nContent-Type: evil.txt", "text/plain", b"d");
    let (_, body) = r.raw.expect("raw body");
    let text = String::from_utf8_lossy(&body).to_string();
    let head = text.split("\r\n\r\n").next().unwrap_or("");
    assert_eq!(
        head.matches("\r\n").count(),
        2,
        "exactly three header lines: {head:?}"
    );
    assert!(!head.contains("evil.txt\r\n"), "{head:?}");
}

#[test]
fn queue_holds_chat_messages_in_order_without_duplicates() {
    let m = |id: &str, session_id: i64| Outgoing {
        client_id: id.into(),
        text: format!("t-{id}"),
        target: Target::Message {
            session_id,
            attachment_ids: vec![],
        },
    };
    let mut q = OutboundQueue::new();
    assert!(q.push(m("a", 1)));
    assert!(q.push(m("b", 1)));
    assert!(!q.push(m("a", 1)));
    assert_eq!(q.pop_front().map(|x| x.client_id), Some("a".into()));
    assert_eq!(q.pop_front().map(|x| x.client_id), Some("b".into()));
}
