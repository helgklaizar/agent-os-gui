//! S6-02: parsing core replies for tools: links, tasks, runs; terminal environment without PATH.

use std::path::Path;

use controllers::terminal_view::user_terminal_env;
use controllers::tool_types::{Link, parse_links, parse_run, parse_runs, parse_task, parse_tasks};
use serde_json::json;

#[test]
fn parse_links_keeps_http_links_with_titles_and_drops_the_rest() {
    let list = json!([
        {"title": "Docs", "url": "https://docs.example.com/"},
        {"title": "Local", "url": "http://localhost:3000"},
        {"title": "Script", "url": "javascript:alert(1)"},
        {"title": "File", "url": "file:///etc/passwd"},
        {"title": "   ", "url": "https://blank.example/"},
        {"title": "No address"},
        {"url": "https://no-title.example/"},
        {"title": 5, "url": "https://x.y/"},
        "string",
    ]);
    let got = parse_links(&list);
    assert_eq!(
        got,
        vec![
            Link {
                title: "Docs".into(),
                url: "https://docs.example.com/".into()
            },
            Link {
                title: "Local".into(),
                url: "http://localhost:3000".into()
            },
        ]
    );
    assert!(
        parse_links(&json!({"title": "x"})).is_empty(),
        "not an array"
    );
    assert!(parse_links(&json!(null)).is_empty());
}

#[test]
fn link_json_has_title_and_url_only() {
    let l = Link {
        title: "T".into(),
        url: "https://x.y/".into(),
    };
    assert_eq!(l.json(), json!({"title": "T", "url": "https://x.y/"}));
}

#[test]
fn parse_task_requires_known_kind_and_state() {
    let ok = json!({"id": 3, "kind": "sched", "title": "t", "spec": "s", "cron": "0 9 * * *", "tz": "Asia/Jerusalem", "state": "active"});
    let t = parse_task(&ok).unwrap_or_else(|| panic!("task not parsed"));
    assert_eq!(
        (t.id, t.kind.as_str(), t.state.as_str()),
        (3, "sched", "active")
    );
    assert_eq!(t.cron.as_deref(), Some("0 9 * * *"));
    assert_eq!(t.tz, "Asia/Jerusalem");
    let no_tz = parse_task(&json!({"id": 1, "kind": "deleg", "title": "t", "state": "paused"}));
    assert_eq!(
        no_tz.map(|t| (t.tz, t.cron)),
        Some(("Asia/Jerusalem".to_string(), None)),
        "default time zone"
    );
    for bad in [
        json!({"id": 1, "kind": "other", "title": "t", "state": "active"}),
        json!({"id": 1, "kind": "sched", "title": "t", "state": "gone"}),
        json!({"id": 1, "kind": "sched", "state": "active"}),
        json!({"kind": "sched", "title": "t", "state": "active"}),
    ] {
        assert!(parse_task(&bad).is_none(), "{bad}");
    }
    let all = parse_tasks(&json!([ok, {"id": 2, "kind": "x"}]));
    assert_eq!(all.len(), 1, "an invalid entry is skipped");
    assert!(parse_tasks(&json!({})).is_empty());
}

#[test]
fn parse_run_reads_status_and_output() {
    let run = parse_run(
        &json!({"id": 4, "task_id": 2, "started_at": 100, "status": "ok", "output": "done"}),
    );
    let r = run.unwrap_or_else(|| panic!("run not parsed"));
    assert_eq!(
        (r.id, r.task_id, r.started_at, r.status.as_str()),
        (4, 2, 100, "ok")
    );
    assert_eq!(r.output.as_deref(), Some("done"));
    assert!(
        parse_run(&json!({"id": 4, "task_id": 2, "status": "ok"})).is_none(),
        "no started_at"
    );
    let many =
        parse_runs(&json!([{"id": 1, "task_id": 1, "started_at": 1, "status": "ok"}, {"id": 2}]));
    assert_eq!(many.len(), 1);
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn user_terminal_env_keeps_given_path_once_and_defaults_only_when_missing() {
    let with = user_terminal_env(
        &env(&[("HOME", "/h"), ("PATH", "/opt/bin")]),
        Path::new("/w"),
    );
    let paths: Vec<&str> = with
        .iter()
        .filter(|(k, _)| k == "PATH")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["/opt/bin"],
        "a given PATH is neither duplicated nor replaced"
    );
    let without = user_terminal_env(&env(&[("HOME", "/h")]), Path::new("/w"));
    let paths: Vec<&str> = without
        .iter()
        .filter(|(k, _)| k == "PATH")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["/usr/local/bin:/usr/bin:/bin"],
        "without PATH: a safe value"
    );
    let nul = user_terminal_env(&env(&[("LANG", "ru\0RU")]), Path::new("/w"));
    assert!(
        !nul.iter().any(|(k, _)| k == "LANG"),
        "a value with NUL is not inherited"
    );
    let lc = user_terminal_env(&env(&[("LC_ALL", "C"), ("LCX", "no")]), Path::new("/w"));
    assert!(lc.iter().any(|(k, _)| k == "LC_ALL") && !lc.iter().any(|(k, _)| k == "LCX"));
}
