//! S8 scenarios through the core client: `fake-agentd` -> `core-client` -> `IndicatorController`.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{client, http, send, wait_event};
use controllers::controllers::indicator::{
    BarView, Frame as IndFrame, IndicatorCfg, IndicatorController, Msg as IndMsg, parse_frame,
};
use controllers::indicators::State;
use controllers::{BusEvent, Controller, Effect};
use core_client::CoreEvent;
use core_client::http::Request;
use fake_agentd::{Config, FakeAgentd};
use serde_json::{Value, json};

fn fresh_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sessions.db");
    drop(fake_agentd::Db::open(&path).expect("DB"));
    (dir, path)
}

async fn start_at(path: &Path) -> FakeAgentd {
    common::start(Config::temp().no_ticker().db(path.to_path_buf())).await
}

fn events_count(path: &Path, indicator: &str) -> i64 {
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| {
            c.query_row(
                "SELECT count(*) FROM events WHERE dedup_key LIKE ?1",
                [format!("{indicator}:%")],
                |r| r.get(0),
            )
        })
        .unwrap_or(-1)
}

/// `bar.indicators` frames from WS -> `parse_frame` -> `IndicatorController`.
async fn pump(
    rx: &async_channel::Receiver<CoreEvent>,
    ctl: &mut IndicatorController,
    n: usize,
    t0: Instant,
) -> Vec<BusEvent> {
    let mut alerts = Vec::new();
    for _ in 0..n {
        let ev = wait_event(
            rx,
            "bar.indicators frame",
            |e| matches!(e, CoreEvent::Frame(f) if f.topic == "bar.indicators"),
        )
        .await
        .expect("frame");
        let CoreEvent::Frame(f) = ev else { continue };
        let items: Vec<Value> = match f.data.get("items") {
            Some(Value::Array(a)) => a.clone(),
            _ => vec![f.data.clone()],
        };
        for item in items {
            let frame = parse_frame(&item).expect("indicator frame parsed");
            for e in ctl.update(IndMsg::Frame(frame), t0 + Duration::from_millis(1)) {
                if let Effect::Publish(ev) = e {
                    alerts.push(ev);
                }
            }
        }
    }
    alerts
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_frames_flow_from_core_and_make_one_event_and_one_alert() {
    let (_dir, path) = fresh_db();
    let d = start_at(&path).await;
    let core = client(&d);
    assert!(core.subscribe("bar.indicators").is_ok());
    let rx = core.events();
    let first = wait_event(
        &rx,
        "snapshot",
        |e| matches!(e, CoreEvent::Frame(f) if f.topic == "bar.indicators"),
    )
    .await
    .expect("snapshot");
    let CoreEvent::Frame(snap) = first else {
        panic!("not a frame")
    };
    let disk = snap.data["items"]
        .as_array()
        .and_then(|a| a.iter().find(|i| i["id"] == "disk").cloned());
    assert_eq!(
        disk.map(|d| (d["state"].clone(), d["transition_seq"].clone())),
        Some((json!("unknown"), json!(0))),
        "bar.indicators snapshot: items with state from the table"
    );

    let admin = http(&d, Some(d.admin_token()));
    for _ in 0..100 {
        let r = send(
            &admin,
            Request::post(
                "/v1/_test/indicator-frame",
                json!({ "id": "disk", "state": "bad" }),
            ),
        )
        .await;
        assert_eq!(r.status, 200);
    }
    let t0 = Instant::now();
    let mut ctl = IndicatorController::default();
    ctl.update(
        IndMsg::Config(vec![IndicatorCfg {
            id: "disk".into(),
            enabled: true,
            position: 0,
        }]),
        t0,
    );
    ctl.update(IndMsg::Width(2000), t0);
    let alerts = pump(&rx, &mut ctl, 100, t0).await;
    assert_eq!(
        alerts,
        vec![BusEvent::IndicatorAlert {
            id: "disk".into(),
            state: State::Bad
        }],
        "one alert per 100 frames"
    );
    assert_eq!(
        events_count(&path, "disk"),
        1,
        "and one event in the core DB"
    );
    let v: BarView = ctl.view();
    assert_eq!(v.visible.first().map(|i| i.state), Some(State::Bad));
}

#[tokio::test(flavor = "multi_thread")]
async fn core_restart_keeps_state_and_core_failure_of_one_source_leaves_others() {
    let (_dir, path) = fresh_db();
    {
        let d = start_at(&path).await;
        let admin = http(&d, Some(d.admin_token()));
        for (id, st) in [("cpu", "bad"), ("ram", "ok")] {
            let r = send(
                &admin,
                Request::post(
                    "/v1/_test/indicator-frame",
                    json!({ "id": id, "state": st }),
                ),
            )
            .await;
            assert_eq!(r.status, 200);
        }
    }
    let d = start_at(&path).await;
    let admin = http(&d, Some(d.admin_token()));
    let r = send(
        &admin,
        Request::post(
            "/v1/_test/indicator-frame",
            json!({ "id": "cpu", "state": "bad" }),
        ),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("JSON");
    assert_eq!(
        v["changed"],
        json!(false),
        "after a restart the core remembers red: no new transition"
    );
    assert_eq!(events_count(&path, "cpu"), 1);

    // failure of one indicator's source: grey, the neighbour works
    let t0 = Instant::now();
    let mut ctl = IndicatorController::default();
    ctl.update(
        IndMsg::Config(vec![
            IndicatorCfg {
                id: "cpu".into(),
                enabled: true,
                position: 0,
            },
            IndicatorCfg {
                id: "ram".into(),
                enabled: true,
                position: 1,
            },
        ]),
        t0,
    );
    ctl.update(
        IndMsg::Frame(IndFrame {
            id: "cpu".into(),
            state: State::Bad,
            seq: 1,
            value: None,
            text: None,
        }),
        t0,
    );
    ctl.update(
        IndMsg::Frame(IndFrame {
            id: "ram".into(),
            state: State::Ok,
            seq: 1,
            value: None,
            text: None,
        }),
        t0,
    );
    ctl.update(IndMsg::SourceFailed("ram".into()), t0);
    let states: Vec<State> = ctl.view().visible.iter().map(|i| i.state).collect();
    assert_eq!(states, vec![State::Bad, State::NoData]);
}
