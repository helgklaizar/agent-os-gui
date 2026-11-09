//! S8-T-09: silent and failed indicator sources, frame parsing.

mod indicator_common;

use controllers::controllers::indicator::{Frame, Msg, parse_frame};
use controllers::indicators::State;
use indicator_common::Rig;
use serde_json::json;

#[test]
fn silent_periodic_source_turns_grey_after_10s_others_keep_working() {
    let mut r = Rig::new();
    r.config(&["net.rate", "ai.cost", "infra.backup"]);
    r.send_at(
        Msg::Frame(Frame {
            id: "net.rate".into(),
            state: State::Ok,
            seq: 1,
            value: Some(1.0),
            text: None,
        }),
        0,
    );
    r.send_at(
        Msg::Frame(Frame {
            id: "ai.cost".into(),
            state: State::Ok,
            seq: 1,
            value: Some(2.0),
            text: None,
        }),
        0,
    );
    r.send_at(
        Msg::Frame(Frame {
            id: "infra.backup".into(),
            state: State::Warn,
            seq: 1,
            value: Some(30.0),
            text: None,
        }),
        0,
    );
    // ai.cost keeps sending frames, net.rate went silent
    r.send_at(
        Msg::Frame(Frame {
            id: "ai.cost".into(),
            state: State::Ok,
            seq: 1,
            value: Some(3.0),
            text: None,
        }),
        9,
    );
    r.send_at(Msg::Tick, 9);
    assert_eq!(
        r.state_of("net.rate"),
        Some(State::Ok),
        "10 s have not passed yet"
    );
    r.send_at(
        Msg::Frame(Frame {
            id: "ai.cost".into(),
            state: State::Ok,
            seq: 1,
            value: Some(4.0),
            text: None,
        }),
        11,
    );
    r.send_at(Msg::Tick, 11);
    assert_eq!(
        r.state_of("net.rate"),
        Some(State::NoData),
        "after 10 s of silence: grey"
    );
    assert_eq!(
        r.state_of("ai.cost"),
        Some(State::Ok),
        "the neighbours work"
    );
    assert_eq!(
        r.state_of("infra.backup"),
        Some(State::Warn),
        "an hourly source is not considered failed after 10 s"
    );
    r.send_at(
        Msg::Frame(Frame {
            id: "net.rate".into(),
            state: State::Ok,
            seq: 1,
            value: Some(5.0),
            text: None,
        }),
        12,
    );
    assert_eq!(
        r.state_of("net.rate"),
        Some(State::Ok),
        "the source came back"
    );
}

#[test]
fn explicit_source_failure_is_grey_immediately_and_isolated() {
    let mut r = Rig::new();
    r.config(&["a", "b"]);
    r.frame("a", State::Ok, 1);
    r.frame("b", State::Bad, 1);
    r.send(Msg::SourceFailed("a".into()));
    assert_eq!(r.state_of("a"), Some(State::NoData));
    assert_eq!(r.state_of("b"), Some(State::Bad));
    r.send(Msg::SourceFailed("not-configured".into()));
    r.frame("not-configured", State::Bad, 1);
    assert_eq!(
        r.view().visible.len(),
        2,
        "foreign ids do not break the bar"
    );
}

#[test]
fn malformed_frames_are_rejected_not_swallowed() {
    let ok = parse_frame(&json!({ "id": "cpu", "state": "warn", "seq": 3, "value": 12.5 }));
    assert_eq!(
        ok,
        Ok(Frame {
            id: "cpu".into(),
            state: State::Warn,
            seq: 3,
            value: Some(12.5),
            text: None
        })
    );
    assert!(
        parse_frame(&json!({ "id": "cpu", "state": "red" })).is_err(),
        "foreign state"
    );
    assert!(parse_frame(&json!({ "state": "ok" })).is_err(), "no id");
    assert!(
        parse_frame(&json!({ "id": "", "state": "ok" })).is_err(),
        "empty id"
    );
    assert!(parse_frame(&json!("string")).is_err());
    let no_seq = parse_frame(&json!({ "id": "cpu", "state": "ok" }));
    assert_eq!(
        no_seq.map(|f| f.seq),
        Ok(0),
        "seq is optional: a source without a counter"
    );
}
