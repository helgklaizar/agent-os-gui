//! GUI side of search: `SearchController`, `SearchHit` parsing, escaping of results.

use std::time::Instant;

use controllers::controllers::search::{Msg, SearchController, Status};
use controllers::hits::{Hit, HitKind, HitSource, HitTarget, parse_hits};
use controllers::testkit::evil_strings;
use controllers::{BusEvent, Controller, CoreCommand, Effect};
use serde_json::json;

fn hit(kind: HitKind, title: &str, target: Option<HitTarget>) -> Hit {
    Hit {
        kind,
        source: HitSource::Local,
        title: title.into(),
        snippet: String::new(),
        target,
    }
}

fn step(c: &mut SearchController, m: Msg) -> Vec<Effect> {
    c.update(m, Instant::now())
}

#[test]
fn query_sends_search_command_and_blank_query_does_not() {
    let mut c = SearchController::default();
    step(&mut c, Msg::Open);
    assert!(c.view().open);
    let fx = step(&mut c, Msg::Query("release".into()));
    assert_eq!(
        fx,
        vec![Effect::Command(CoreCommand::Search {
            query: "release".into(),
            limit: 20
        })]
    );
    assert_eq!(c.view().status, Status::Loading);
    assert!(
        step(&mut c, Msg::Query("   ".into())).is_empty(),
        "an empty query is not sent to the core"
    );
    assert_eq!(c.view().status, Status::Idle);
    assert!(c.view().groups.is_empty());
}

#[test]
fn results_are_grouped_in_d16_order_and_stale_answers_dropped() {
    let mut c = SearchController::default();
    step(&mut c, Msg::Open);
    step(&mut c, Msg::Query("a".into()));
    step(&mut c, Msg::Query("ab".into()));
    step(
        &mut c,
        Msg::Results {
            query: "a".into(),
            hits: vec![hit(HitKind::File, "old", None)],
        },
    );
    assert!(
        c.view().groups.is_empty(),
        "the answer to a previous query is dropped"
    );
    step(
        &mut c,
        Msg::Results {
            query: "ab".into(),
            hits: vec![
                hit(HitKind::Memory, "m", None),
                hit(HitKind::Session, "s1", None),
                hit(HitKind::Mail, "mail", None),
                hit(HitKind::Session, "s2", None),
                hit(HitKind::File, "f", None),
                hit(HitKind::Message, "msg", None),
            ],
        },
    );
    let v = c.view();
    assert_eq!(v.status, Status::Done);
    let kinds: Vec<HitKind> = v.groups.iter().map(|g| g.kind).collect();
    assert_eq!(
        kinds,
        vec![
            HitKind::Session,
            HitKind::Message,
            HitKind::Mail,
            HitKind::File,
            HitKind::Memory
        ]
    );
    assert_eq!(
        v.groups[0]
            .hits
            .iter()
            .map(|h| h.title.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s2"]
    );
    assert_eq!(v.selected, Some(0), "the first result is selected");
}

#[test]
fn move_wraps_and_enter_opens_target_and_closes_palette() {
    let mut c = SearchController::default();
    step(&mut c, Msg::Open);
    step(&mut c, Msg::Query("x".into()));
    let target = HitTarget::Session {
        session_id: 7,
        message_id: Some(3),
    };
    step(
        &mut c,
        Msg::Results {
            query: "x".into(),
            hits: vec![
                hit(
                    HitKind::Session,
                    "s",
                    Some(HitTarget::Session {
                        session_id: 1,
                        message_id: None,
                    }),
                ),
                hit(HitKind::Message, "m", Some(target.clone())),
            ],
        },
    );
    step(&mut c, Msg::Move(1));
    assert_eq!(c.view().selected, Some(1));
    step(&mut c, Msg::Move(1));
    assert_eq!(c.view().selected, Some(0), "wraps around downwards");
    step(&mut c, Msg::Move(-1));
    assert_eq!(c.view().selected, Some(1), "wraps around upwards");
    let fx = step(&mut c, Msg::Activate);
    assert_eq!(fx, vec![Effect::Publish(BusEvent::OpenHit(target))]);
    assert!(!c.view().open, "the palette is closed after navigating");
}

#[test]
fn enter_without_target_or_results_does_nothing() {
    let mut c = SearchController::default();
    step(&mut c, Msg::Open);
    assert!(step(&mut c, Msg::Activate).is_empty());
    step(&mut c, Msg::Query("x".into()));
    step(
        &mut c,
        Msg::Results {
            query: "x".into(),
            hits: vec![hit(HitKind::Memory, "no ref", None)],
        },
    );
    assert!(step(&mut c, Msg::Activate).is_empty());
    assert!(c.view().open, "the palette stayed open");
}

#[test]
fn close_resets_state_and_failure_is_visible() {
    let mut c = SearchController::default();
    step(&mut c, Msg::Open);
    step(&mut c, Msg::Query("x".into()));
    step(&mut c, Msg::Failed { query: "x".into() });
    assert_eq!(
        c.view().status,
        Status::Error,
        "an error is not masked by an empty list"
    );
    step(&mut c, Msg::Close);
    let v = c.view();
    assert!(!v.open && v.query.is_empty() && v.groups.is_empty() && v.status == Status::Idle);
    step(&mut c, Msg::Failed { query: "x".into() });
    assert_eq!(
        c.view().status,
        Status::Idle,
        "a late failure for a closed palette is ignored"
    );
}

#[test]
fn parse_hits_reads_source_and_targets() {
    let body = json!({ "items": [
        { "kind": "session", "source": "local", "title": "S", "snippet": "", "ref": { "session_id": 4 } },
        { "kind": "message", "source": "local", "title": "M", "snippet": "t", "ref": { "session_id": 4, "message_id": 9 } },
        { "kind": "mail", "source": "local", "title": "P", "ref": { "id": 2 } },
        { "kind": "file", "source": "local", "title": "F", "ref": { "path": "notes/a.md" } },
        { "kind": "memory", "source": "mcp_memory", "title": "Pm", "ref": { "id": "doc-1" } },
        { "kind": "memory", "source": "local", "title": "no ref" }
    ]});
    let hits = parse_hits(&body).expect("parse");
    assert_eq!(hits.len(), 6);
    assert_eq!(
        hits[0].target,
        Some(HitTarget::Session {
            session_id: 4,
            message_id: None
        })
    );
    assert_eq!(
        hits[1].target,
        Some(HitTarget::Session {
            session_id: 4,
            message_id: Some(9)
        })
    );
    assert_eq!(hits[2].target, Some(HitTarget::Mail { id: 2 }));
    assert_eq!(
        hits[3].target,
        Some(HitTarget::File {
            path: "notes/a.md".into()
        })
    );
    assert_eq!(hits[4].source, HitSource::McpMemory);
    assert_eq!(
        hits[4].target,
        Some(HitTarget::Memory { id: "doc-1".into() })
    );
    assert_eq!(
        (hits[5].source, hits[5].target.clone()),
        (HitSource::Local, None),
        "source local is read, no ref"
    );
}

#[test]
fn parse_hits_rejects_garbage_instead_of_dropping_it() {
    for bad in [
        json!({}),
        json!({ "items": "x" }),
        json!({ "items": [{ "kind": "video", "title": "x" }] }),
        json!({ "items": [{ "kind": "file" }] }),
        json!({ "items": [{ "kind": "file", "title": "x", "source": "cloud" }] }),
        json!([]),
    ] {
        assert!(parse_hits(&bad).is_err(), "{bad}");
    }
}

#[test]
fn parse_hits_requires_source_on_every_item() {
    // docs/reference/api.md section 5.5: `source` is required; a missing or null value is an error for the whole response, not a silent `local`.
    for kind in ["session", "message", "mail", "file", "memory"] {
        let missing = json!({ "items": [{ "kind": kind, "title": "x" }] });
        let err = parse_hits(&missing).expect_err("without source");
        assert!(err.to_string().contains("items[0].source"), "{kind}: {err}");
        let null = json!({ "items": [{ "kind": kind, "title": "x", "source": null }] });
        assert!(parse_hits(&null).is_err(), "{kind}: null instead of source");
    }
    let second = json!({ "items": [
        { "kind": "file", "source": "local", "title": "a" },
        { "kind": "file", "title": "b" }
    ]});
    let err = parse_hits(&second).expect_err("second without source");
    assert!(err.to_string().contains("items[1].source"), "{err}");
    for bad in [json!(1), json!(""), json!("LOCAL"), json!(["local"])] {
        let body = json!({ "items": [{ "kind": "file", "title": "x", "source": bad }] });
        assert!(parse_hits(&body).is_err(), "{bad}");
    }
    let ok = json!({ "items": [
        { "kind": "memory", "source": "local", "title": "a" },
        { "kind": "memory", "source": "mcp_memory", "title": "b" }
    ]});
    let hits = parse_hits(&ok).expect("both sources accepted");
    assert_eq!(
        hits.iter().map(|h| h.source).collect::<Vec<_>>(),
        vec![HitSource::Local, HitSource::McpMemory]
    );
}

#[test]
fn hostile_text_is_escaped_for_markup() {
    for evil in evil_strings() {
        let h = Hit {
            kind: HitKind::Message,
            source: HitSource::Local,
            title: evil.clone(),
            snippet: evil.clone(),
            target: None,
        };
        for m in [h.title_markup(), h.snippet_markup()] {
            assert!(
                !m.contains('<') && !m.contains('>'),
                "raw markup leaked through: {m:.60}"
            );
            assert!(!m.contains('"') && !m.contains('\''));
            assert!(!evil.is_empty() && !m.is_empty());
        }
    }
    let h = hit(HitKind::File, "a & b", None);
    assert_eq!(h.title_markup(), "a &amp; b");
}
