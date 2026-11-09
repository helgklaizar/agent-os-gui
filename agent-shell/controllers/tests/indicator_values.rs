//! S8-02…S8-05: the value and tooltip details from the frame reach the bar view.

mod indicator_common;

use controllers::controllers::indicator::{Frame, ItemView, Msg, parse_frame};
use controllers::indicators::State;
use indicator_common::Rig;
use serde_json::json;

fn frame(id: &str, state: State, seq: u64, value: Option<f64>, text: Option<&str>) -> Msg {
    Msg::Frame(Frame {
        id: id.into(),
        state,
        seq,
        value,
        text: text.map(str::to_string),
    })
}

fn item(r: &Rig, id: &str) -> Option<ItemView> {
    r.view().visible.into_iter().find(|i| i.id == id)
}

#[test]
fn value_and_tooltip_text_reach_the_view_and_survive_click() {
    let mut r = Rig::new();
    r.config(&["disk", "ram"]);
    r.send(frame(
        "disk",
        State::Warn,
        1,
        Some(82.5),
        Some("/ used 82.5%"),
    ));
    let d = item(&r, "disk").expect("disk in the bar");
    assert_eq!(d.value, Some(82.5));
    assert_eq!(d.detail.as_deref(), Some("/ used 82.5%"));
    assert_eq!(
        item(&r, "ram").and_then(|i| i.value),
        None,
        "a foreign frame is not visible"
    );
    r.send(Msg::Click("disk".into()));
    let d = item(&r, "disk").expect("disk");
    assert!(d.tooltip_open);
    assert_eq!(
        d.detail.as_deref(),
        Some("/ used 82.5%"),
        "the tooltip is open with details"
    );
}

#[test]
fn newer_frame_replaces_value_and_a_frame_without_value_clears_it() {
    let mut r = Rig::new();
    r.config(&["disk"]);
    r.send(frame("disk", State::Warn, 1, Some(82.5), Some("a")));
    r.send(frame("disk", State::Bad, 2, Some(91.0), Some("b")));
    let d = item(&r, "disk").expect("disk");
    assert_eq!((d.value, d.detail.as_deref()), (Some(91.0), Some("b")));
    r.send(frame("disk", State::Bad, 3, None, None));
    let d = item(&r, "disk").expect("disk");
    assert_eq!((d.value, d.detail), (None, None));
}

#[test]
fn failed_source_loses_value_and_details() {
    let mut r = Rig::new();
    r.config(&["disk"]);
    r.send(frame("disk", State::Bad, 1, Some(91.0), Some("b")));
    r.send(Msg::SourceFailed("disk".into()));
    let d = item(&r, "disk").expect("disk");
    assert_eq!(d.state, State::NoData);
    assert_eq!(
        (d.value, d.detail),
        (None, None),
        "a grey indicator does not show the old value"
    );
}

#[test]
fn parse_frame_reads_text_and_rejects_wrong_or_oversized_text() {
    let ok =
        parse_frame(&json!({ "id": "disk", "state": "warn", "value": 82.5, "text": "/ 82.5%" }));
    assert_eq!(ok.map(|f| f.text), Ok(Some("/ 82.5%".to_string())));
    assert!(parse_frame(&json!({ "id": "disk", "state": "ok", "text": 5 })).is_err());
    let long = "é".repeat(501);
    assert!(parse_frame(&json!({ "id": "disk", "state": "ok", "text": long })).is_err());
    let max = "é".repeat(500);
    assert!(parse_frame(&json!({ "id": "disk", "state": "ok", "text": max })).is_ok());
    let none = parse_frame(&json!({ "id": "disk", "state": "ok", "text": null }));
    assert_eq!(none.map(|f| f.text), Ok(None));
}
