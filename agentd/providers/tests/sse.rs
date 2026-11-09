//! SSE parsing: events do not depend on how the stream is chunked.

use agentd_providers::sse::{SseEvent, SseParser};

fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
    let mut p = SseParser::new();
    chunks.iter().flat_map(|c| p.feed(c)).collect()
}

fn ev(event: Option<&str>, data: &str) -> SseEvent {
    SseEvent {
        event: event.map(str::to_string),
        data: data.to_string(),
    }
}

#[test]
fn sse_basic_events() {
    let got = parse_all(&[b"event: a\ndata: 1\n\ndata: 2\n\n"]);
    assert_eq!(got, [ev(Some("a"), "1"), ev(None, "2")]);
}

#[test]
fn sse_crlf_comments_and_multiline_data() {
    let got = parse_all(&[b": keep-alive\r\nevent: x\r\ndata: a\r\ndata: b\r\n\r\n"]);
    assert_eq!(got, [ev(Some("x"), "a\nb")]);
}

#[test]
fn sse_split_at_every_byte_gives_same_events() {
    let text = "event: e\ndata: Hello, world 🔑\n\ndata: [DONE]\n\n".as_bytes();
    let whole = parse_all(&[text]);
    assert_eq!(whole.len(), 2);
    let bytes: Vec<&[u8]> = text.chunks(1).collect();
    assert_eq!(
        parse_all(&bytes),
        whole,
        "byte-by-byte chunking, including inside UTF-8"
    );
}

#[test]
fn sse_incomplete_event_is_not_emitted() {
    let mut p = SseParser::new();
    assert!(p.feed(b"data: partial\n").is_empty());
    assert!(p.feed(b"data: more").is_empty());
    assert_eq!(p.feed(b"\n\n"), [ev(None, "partial\nmore")]);
}

#[test]
fn sse_data_without_space_and_empty_value() {
    let got = parse_all(&[b"data:x\n\ndata:\n\n"]);
    assert_eq!(got, [ev(None, "x"), ev(None, "")]);
}
