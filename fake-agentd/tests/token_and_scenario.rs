//! Token format, SHA-256 and TOML scenario parsing.

use std::collections::HashSet;
use std::path::PathBuf;

use fake_agentd::FrameType;
use fake_agentd::scenario::{Action, Scenario, StepKind};
use fake_agentd::token::{generate, is_valid_format, sha256_hex};

#[test]
fn token_format_matches_generator() {
    let t = generate();
    assert!(t.starts_with("agt_"), "prefix agt_");
    assert_eq!(
        t.len(),
        4 + 43,
        "32 bytes in base64url without padding give 43 characters"
    );
    assert!(is_valid_format(&t));
    let seen: HashSet<String> = (0..500).map(|_| generate()).collect();
    assert_eq!(seen.len(), 500, "tokens do not repeat");
    assert!(seen.iter().all(|t| is_valid_format(t)));
}

#[test]
fn token_format_rejects_foreign() {
    let good = generate();
    let body = &good[good.len().min(4)..];
    let short = &body[..body.len().min(42)];
    for bad in [
        String::new(),
        "agt_".to_string(),
        format!("AGT_{body}"),
        format!("tok_{body}"),
        format!("agt_{short}"),
        format!("agt_{body}A"),
        format!("agt_{short}+"),
        format!("agt_{short}/"),
        format!("agt_{short}="),
        format!("agt_{short} "),
        format!("{good}\n"),
    ] {
        assert!(!is_valid_format(&bad), "foreign format accepted: {bad:?}");
    }
    assert!(is_valid_format(&good), "own format accepted");
}

#[test]
fn sha256_hex_known_vector() {
    assert_eq!(
        sha256_hex("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let h = sha256_hex(&generate());
    assert_eq!(h.len(), 64);
    assert!(
        h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
}

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scenarios")
}

#[test]
fn scenario_bar_then_drop_parses() {
    let s = Scenario::from_file(&scenarios_dir().join("bar-then-drop.toml"));
    let Ok(s) = s else {
        panic!("scenario not parsed: {s:?}")
    };
    assert_eq!(s.name, "bar-then-drop");
    assert_eq!(s.steps.len(), 3);
    assert_eq!(s.steps[0].at_ms, 0);
    assert!(
        matches!(&s.steps[0].kind, StepKind::Emit { topic, kind: FrameType::Delta, data } if topic == "bar.sys" && data["cpu"] == 14)
    );
    assert_eq!(
        (s.steps[1].at_ms, &s.steps[1].kind),
        (5000, &StepKind::Action(Action::DropWs))
    );
    assert_eq!(
        (s.steps[2].at_ms, &s.steps[2].kind),
        (7000, &StepKind::Action(Action::Restore))
    );
}

#[test]
fn scenario_idle_parses_and_has_no_steps() {
    let s = Scenario::from_file(&scenarios_dir().join("idle.toml"));
    assert!(
        matches!(&s, Ok(s) if s.name == "idle" && s.steps.is_empty()),
        "{s:?}"
    );
}

#[test]
fn scenario_actions_and_emit_types() {
    let text = r#"
name = "x"
[[step]]
at_ms = 10
action = "no_data:bar.sys"
[[step]]
at_ms = 20
emit = { topic = "events", type = "event", data = { id = 1 } }
"#;
    let s = Scenario::from_toml(text);
    let Ok(s) = s else { panic!("{s:?}") };
    assert_eq!(s.steps.len(), 2);
    assert_eq!(
        s.steps[0].kind,
        StepKind::Action(Action::NoData("bar.sys".into()))
    );
    assert!(matches!(
        &s.steps[1].kind,
        StepKind::Emit {
            kind: FrameType::Event,
            ..
        }
    ));
}

#[test]
fn scenario_rejects_bad_input() {
    for (name, text) in [
        (
            "unknown action",
            "name=\"x\"\n[[step]]\nat_ms=1\naction=\"explode\"",
        ),
        ("no at_ms", "name=\"x\"\n[[step]]\naction=\"drop_ws\""),
        ("neither emit nor action", "name=\"x\"\n[[step]]\nat_ms=1"),
        (
            "both emit and action",
            "name=\"x\"\n[[step]]\nat_ms=1\naction=\"drop_ws\"\nemit={topic=\"a\",data={}}",
        ),
        (
            "no_data without a topic",
            "name=\"x\"\n[[step]]\nat_ms=1\naction=\"no_data:\"",
        ),
        (
            "unknown frame type",
            "name=\"x\"\n[[step]]\nat_ms=1\nemit={topic=\"a\",type=\"weird\",data={}}",
        ),
        ("not TOML", "this is not toml ["),
        (
            "steps out of order",
            "name=\"x\"\n[[step]]\nat_ms=5\naction=\"drop_ws\"\n[[step]]\nat_ms=1\naction=\"restore\"",
        ),
    ] {
        assert!(Scenario::from_toml(text).is_err(), "accepted: {name}");
    }
}
