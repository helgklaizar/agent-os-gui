//! S8 search through the core client: `fake-agentd` -> `core-client` -> `SearchController`.

mod common;
#[path = "../../../fake-agentd/tests/common/fixture.rs"]
mod fixture;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use common::{http, issue, send};
use controllers::controllers::search::{Msg as SearchMsg, SearchController};
use controllers::hits::{HitKind, HitSource, HitTarget, parse_hits};
use controllers::{BusEvent, Controller, CoreCommand, Effect, EventBus};
use core_client::http::Request;
use fake_agentd::{Config, FakeAgentd, Kind, Scope};
use serde_json::{Value, json};

fn db_with_baseline(seed: bool) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sessions.db");
    let db = fake_agentd::Db::open(&path).expect("DB");
    if seed {
        fixture::seed_search(&db);
    }
    (dir, path)
}

async fn start_at(path: &Path) -> FakeAgentd {
    common::start(Config::temp().no_ticker().db(path.to_path_buf())).await
}

fn enc(q: &str) -> String {
    q.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn palette_finds_and_opens_session_message_mail_and_memory_fact() {
    let (_dir, path) = db_with_baseline(true);
    let d = start_at(&path).await;
    let c = http(&d, Some(&issue(&d, Scope::Read, Kind::Test)));
    let bus = EventBus::new();
    let opened: Rc<RefCell<Vec<BusEvent>>> = Rc::default();
    let sink = opened.clone();
    bus.subscribe(move |e| sink.borrow_mut().push(e.clone()));

    let cases: [(&str, HitKind, HitTarget); 4] = [
        (
            "Startup",
            HitKind::Session,
            HitTarget::Session {
                session_id: 1,
                message_id: None,
            },
        ),
        (
            "migration",
            HitKind::Message,
            HitTarget::Session {
                session_id: 1,
                message_id: Some(1),
            },
        ),
        ("hosting", HitKind::Mail, HitTarget::Mail { id: 1 }),
        (
            "short",
            HitKind::Memory,
            HitTarget::Memory { id: "1".into() },
        ),
    ];
    for (query, kind, target) in cases {
        let mut ctl = SearchController::default();
        let t = Instant::now();
        ctl.update(SearchMsg::Open, t);
        let fx = ctl.update(SearchMsg::Query(query.into()), t);
        let Some(Effect::Command(CoreCommand::Search { query: q, limit })) = fx.into_iter().next()
        else {
            panic!("the query did not turn into a core command: {query}");
        };
        let resp = send(
            &c,
            Request::get(&format!("/v1/search?q={}&limit={limit}", enc(&q))),
        )
        .await;
        assert_eq!(resp.status, 200, "{query}");
        let body: Value = serde_json::from_slice(&resp.body).expect("JSON");
        let hits = parse_hits(&body).expect("response parsed by the client");
        assert!(hits.iter().all(|h| h.source == HitSource::Local));
        ctl.update(SearchMsg::Results { query: q, hits }, t);
        assert!(
            ctl.view().groups.iter().any(|g| g.kind == kind),
            "{query}: group {kind:?}"
        );
        // choose a result of the needed kind
        let flat: Vec<HitKind> = ctl
            .view()
            .groups
            .iter()
            .flat_map(|g| g.hits.iter().map(|h| h.kind))
            .collect();
        let idx = flat
            .iter()
            .position(|k| *k == kind)
            .expect("there is a result of the kind");
        for _ in 0..idx {
            ctl.update(SearchMsg::Move(1), t);
        }
        for e in ctl.update(SearchMsg::Activate, t) {
            if let Effect::Publish(ev) = e {
                bus.publish(ev);
            }
        }
        assert!(!ctl.view().open, "the palette is closed after Enter");
        assert_eq!(
            opened.borrow().last(),
            Some(&BusEvent::OpenHit(target)),
            "{query}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_switch_is_visible_to_the_client() {
    let (_dir, path) = db_with_baseline(true);
    let d = start_at(&path).await;
    let admin = http(&d, Some(d.admin_token()));
    let page = send(&admin, Request::get("/v1/settings/memory")).await;
    let rev = serde_json::from_slice::<Value>(&page.body).expect("JSON")["rev"]
        .as_i64()
        .unwrap_or(-1);
    let set = send(
        &admin,
        Request::patch("/v1/settings/memory", json!({ "source": "mcp_memory" }))
            .header("If-Match", &format!("\"{rev}\"")),
    )
    .await;
    assert_eq!(set.status, 200);
    let resp = send(
        &admin,
        Request::get(&format!("/v1/search?q={}", enc("deploy"))),
    )
    .await;
    let hits =
        parse_hits(&serde_json::from_slice::<Value>(&resp.body).expect("JSON")).expect("parse");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].kind, hits[0].source),
        (HitKind::Memory, HitSource::McpMemory)
    );
}
