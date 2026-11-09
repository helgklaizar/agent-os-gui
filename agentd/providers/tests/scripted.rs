//! Scenarios of the `scripted` test provider (`test-support` build only).
#![cfg(feature = "test-support")]

mod common;

use std::time::{Duration, Instant};

use agentd_core::{ModelProvider, ProviderEvent, ProviderKind, Turn, TurnRole};
use agentd_providers::scripted::Scripted;
use common::{account, collect, request, text_of};

fn req(model: &str, text: &str) -> agentd_core::ModelRequest {
    let mut r = request(account(ProviderKind::Scripted, "", None));
    r.model = model.into();
    r.history = vec![Turn {
        role: TurnRole::User,
        text: text.into(),
    }];
    r
}

#[tokio::test]
async fn scripted_default_chunks_and_usage() {
    let ev = collect(&Scripted::new(), req("scripted", "hello")).await;
    assert_eq!(text_of(&ev), "Reply: hello");
    assert!(
        ev.iter()
            .filter(|e| matches!(e, ProviderEvent::Delta(_)))
            .count()
            >= 2,
        "chunks"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, ProviderEvent::Usage(u) if u.tokens_in > 0 && u.tokens_out > 0))
    );
    assert!(
        !ev.iter()
            .any(|e| matches!(e, ProviderEvent::Error(_) | ProviderEvent::RateLimit(_)))
    );
}

#[tokio::test]
async fn scripted_long_is_100k_chars() {
    let ev = collect(&Scripted::new(), req("scripted:long", "x")).await;
    assert_eq!(text_of(&ev).chars().count(), 100_000);
}

#[tokio::test]
async fn scripted_html_verbatim() {
    let ev = collect(&Scripted::new(), req("scripted:html", "x")).await;
    let t = text_of(&ev);
    assert!(
        t.contains("<script>alert(1)</script>") && t.contains("**bold**") && t.contains("<b>"),
        "{t}"
    );
}

#[tokio::test]
async fn scripted_drop_breaks_midstream() {
    let ev = collect(&Scripted::new(), req("scripted:drop", "x")).await;
    assert!(!text_of(&ev).is_empty(), "part of the text arrived");
    let Some(ProviderEvent::Error(e)) = ev.last() else {
        panic!("{ev:?}")
    };
    assert_eq!((e.code, e.retryable), ("network", true));
}

#[tokio::test]
async fn scripted_rate_limited_error_only() {
    let ev = collect(&Scripted::new(), req("scripted:rate_limited", "x")).await;
    let [ProviderEvent::Error(e)] = ev.as_slice() else {
        panic!("{ev:?}")
    };
    assert_eq!((e.code, e.retryable), ("rate_limited", true));
}

#[tokio::test]
async fn scripted_slow_waits_2_5_seconds() {
    let t = Instant::now();
    let ev = collect(&Scripted::new(), req("scripted:slow", "x")).await;
    assert!(
        t.elapsed() >= Duration::from_millis(2500),
        "{:?}",
        t.elapsed()
    );
    assert!(!text_of(&ev).is_empty());
}

#[tokio::test]
async fn scripted_hang_is_cancelled_by_drop() {
    let p = Scripted::new();
    let r = tokio::time::timeout(
        Duration::from_millis(300),
        collect(&p, req("scripted:hang", "x")),
    )
    .await;
    assert!(r.is_err(), "the scenario does not end by itself");
}

#[tokio::test]
async fn scripted_limits_scenarios() {
    for (model, remaining) in [("scripted:limits", 80), ("scripted:limits_low", 5)] {
        let ev = collect(&Scripted::new(), req(model, "x")).await;
        let rl = ev
            .iter()
            .find_map(|e| match e {
                ProviderEvent::RateLimit(r) => Some(*r),
                _ => None,
            })
            .expect(model);
        let w = rl.requests.unwrap();
        assert_eq!((w.limit, w.remaining), (100, remaining), "{model}");
        assert!(w.reset_at.is_some());
    }
}

fn tool_call(ev: &[ProviderEvent]) -> Option<agentd_core::ToolCall> {
    ev.iter().find_map(|e| match e {
        ProviderEvent::ToolCall(c) => Some(c.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn scripted_action_tool_calls() {
    for a in [
        "shell", "send", "delete", "pay", "browse", "read", "config", "danger",
    ] {
        let ev = collect(&Scripted::new(), req(&format!("scripted:action_{a}"), "x")).await;
        let c = tool_call(&ev).unwrap_or_else(|| panic!("{a}: {ev:?}"));
        let want = match a {
            "danger" => "shell",
            "config" => "config_write",
            other => other,
        };
        assert_eq!(c.name, want, "{a}");
        assert!(
            c.args["target"].as_str().is_some_and(|s| !s.is_empty()),
            "{a}"
        );
        assert!(!c.id.is_empty());
    }
    let danger = collect(&Scripted::new(), req("scripted:action_danger", "x")).await;
    assert!(
        tool_call(&danger).unwrap().args["target"]
            .as_str()
            .unwrap()
            .contains("rm -rf")
    );
    // Tool result received: no more requests, the model answers in plain text with the result.
    let ev = collect(
        &Scripted::new(),
        req("scripted:action_send", "[tool_result {\"ok\":false}]"),
    )
    .await;
    assert!(tool_call(&ev).is_none());
    assert!(text_of(&ev).contains("[tool_result"));
}

#[tokio::test]
async fn scripted_scenario_from_message_prefix() {
    let ev = collect(&Scripted::new(), req("scripted", "#scn:rate_limited rest")).await;
    assert!(matches!(ev.as_slice(), [ProviderEvent::Error(_)]));
}

#[tokio::test]
async fn scripted_check_ok() {
    let a = account(ProviderKind::Scripted, "", None);
    Scripted::new().check(&a).await.unwrap();
}

#[tokio::test]
async fn scripted_trickle_and_context() {
    let ev = collect(&Scripted::new(), req("scripted:trickle", "x")).await;
    assert!(
        ev.iter()
            .filter(|e| matches!(e, ProviderEvent::Delta(_)))
            .count()
            >= 20
    );
    let mut r = req("scripted:context", "second");
    r.system = Some("rules".into());
    r.history = vec![
        Turn {
            role: TurnRole::User,
            text: "first".into(),
        },
        Turn {
            role: TurnRole::Assistant,
            text: "answer".into(),
        },
        Turn {
            role: TurnRole::User,
            text: "second".into(),
        },
    ];
    assert_eq!(
        text_of(&collect(&Scripted::new(), r).await),
        "SYS[rules] TURNS[u:first|a:answer|u:second]"
    );
}
