//! Pure parts of the events-feed client: request path and the "mark read" body.

use core_client::events::{EventQuery, EventsRead};
use serde_json::json;

#[test]
fn event_query_empty_is_bare_path() {
    assert_eq!(EventQuery::default().path(), "/v1/events");
}

#[test]
fn event_query_includes_only_set_fields_in_stable_order() {
    let q = EventQuery {
        kind: Some("msg".into()),
        sub: Some("wa".into()),
        unread: Some(true),
        session_id: Some(7),
        before: Some(40),
        limit: Some(50),
    };
    assert_eq!(
        q.path(),
        "/v1/events?kind=msg&sub=wa&unread=true&session_id=7&before=40&limit=50"
    );
    let only_limit = EventQuery {
        limit: Some(200),
        ..EventQuery::default()
    };
    assert_eq!(only_limit.path(), "/v1/events?limit=200");
}

#[test]
fn event_query_encodes_sub_so_it_cannot_inject_parameters() {
    let q = EventQuery {
        sub: Some("a&kind=ai #é".into()),
        ..EventQuery::default()
    };
    assert_eq!(q.path(), "/v1/events?sub=a%26kind%3Dai%20%23%C3%A9");
}

#[test]
fn events_read_body_has_only_set_fields() {
    assert_eq!(EventsRead::default().body(), json!({}));
    let f = EventsRead {
        kind: Some("ai".into()),
        sub: Some("watch".into()),
        session_id: Some(3),
        ids: Some(vec![1, 2]),
    };
    assert_eq!(
        f.body(),
        json!({"kind": "ai", "sub": "watch", "session_id": 3, "ids": [1, 2]})
    );
    let s = EventsRead {
        session_id: Some(9),
        ..EventsRead::default()
    };
    assert_eq!(s.body(), json!({"session_id": 9}));
}
