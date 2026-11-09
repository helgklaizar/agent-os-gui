//! S6-04, S6-06: `BrowserController`: start page, navigation, "Home", per-session profiles and bookmarks,
//! quick links, agent commands.

mod tree_common;

use std::path::Path;
use std::time::Instant;

use controllers::browser_view::{BrowserMsg, agent_profile_dir, profile_dir};
use controllers::controllers::browser::BrowserController;
use controllers::tool_types::{BrowserCommand, Link, ToolResult};
use controllers::{Controller, CoreCommand, Effect};

fn send(c: &mut BrowserController, m: BrowserMsg) -> Vec<Effect> {
    c.update(m, Instant::now())
}

fn link(title: &str, url: &str) -> Link {
    Link {
        title: title.into(),
        url: url.into(),
    }
}

fn open(c: &mut BrowserController, session: i64) -> Vec<Effect> {
    send(c, BrowserMsg::Session(Some(session)))
}

fn go(c: &mut BrowserController, text: &str) -> Vec<Effect> {
    send(c, BrowserMsg::Submit(text.into()))
}

fn commands(fx: &[Effect]) -> Vec<&CoreCommand> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn browser_starts_on_start_page_and_fetches_links() {
    let mut c = BrowserController::new();
    let v = c.view();
    assert!(v.start_page && v.address.is_empty() && v.load.is_none());
    let fx = open(&mut c, 1);
    let cmds = commands(&fx);
    assert!(cmds.contains(&&CoreCommand::FetchQuickLinks), "{fx:?}");
    assert!(cmds.contains(&&CoreCommand::FetchBookmarks { session_id: 1 }));
    assert!(
        c.view().start_page,
        "the start page is visible without a network"
    );
    let again = open(&mut c, 1);
    assert!(
        commands(&again).is_empty(),
        "reopening the same session does not hit the core"
    );
}

#[test]
fn browser_submit_navigates_and_updates_address() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    go(&mut c, "example.com");
    let v = c.view();
    assert!(!v.start_page);
    assert_eq!(v.address, "https://example.com");
    let load = v.load.unwrap_or_else(|| panic!("no load request"));
    assert_eq!(
        (load.session_id, load.url.as_str()),
        (1, "https://example.com")
    );
    let first_seq = load.seq;
    go(&mut c, "rust lang");
    let v = c.view();
    assert!(
        v.address
            .starts_with("https://www.google.com/search?q=rust%20lang"),
        "{}",
        v.address
    );
    assert!(
        v.load.as_ref().is_some_and(|l| l.seq > first_seq),
        "seq grows on every navigation"
    );
    let before = c.view();
    go(&mut c, "   ");
    assert_eq!(c.view(), before, "empty input does nothing");
}

#[test]
fn browser_submit_without_session_is_ignored() {
    let mut c = BrowserController::new();
    let fx = go(&mut c, "example.com");
    assert!(fx.is_empty());
    assert!(c.view().load.is_none());
}

#[test]
fn browser_back_forward_reload_follow_history() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    go(&mut c, "a.example.com");
    go(&mut c, "b.example.com");
    assert!(c.view().can_back && !c.view().can_forward);
    send(&mut c, BrowserMsg::Back);
    let v = c.view();
    assert_eq!(v.address, "https://a.example.com");
    assert_eq!(
        v.load.as_ref().map(|l| l.url.as_str()),
        Some("https://a.example.com")
    );
    assert!(v.can_forward);
    send(&mut c, BrowserMsg::Forward);
    assert_eq!(c.view().address, "https://b.example.com");
    let seq = c.view().load.map_or(0, |l| l.seq);
    send(&mut c, BrowserMsg::Reload);
    let v = c.view();
    assert_eq!(v.address, "https://b.example.com");
    assert!(v.load.is_some_and(|l| l.seq > seq), "reload is a new load");
    send(&mut c, BrowserMsg::Back);
    send(&mut c, BrowserMsg::Back);
    assert!(
        c.view().start_page,
        "back from the first page is the start page"
    );
    assert!(!c.view().can_back);
}

#[test]
fn browser_loaded_events_extend_history_and_sanitize_title() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    go(&mut c, "a.example.com");
    send(
        &mut c,
        BrowserMsg::Loaded {
            session_id: 1,
            url: "https://a.example.com/next".into(),
            title: "<b>Heading</b>\u{7}".into(),
        },
    );
    let v = c.view();
    assert_eq!(
        v.address, "https://a.example.com/next",
        "following a link went into the history"
    );
    assert_eq!(
        v.title, "<b>Heading</b>",
        "title without control characters, markup as literal text"
    );
    assert!(v.can_back);
    send(
        &mut c,
        BrowserMsg::Loaded {
            session_id: 1,
            url: "javascript:alert(1)".into(),
            title: "x".into(),
        },
    );
    assert_eq!(
        c.view().address,
        "https://a.example.com/next",
        "a foreign scheme is not accepted"
    );
    send(
        &mut c,
        BrowserMsg::Loaded {
            session_id: 1,
            url: "https://a.example.com/next".into(),
            title: "t".repeat(1000),
        },
    );
    assert!(
        c.view().title.chars().count() <= 201,
        "the title is bounded"
    );
}

#[test]
fn browser_home_does_not_reset_app_state() {
    let mut tree = tree_common::loaded();
    let rows_before = {
        use controllers::Controller;
        tree.view().rows.len()
    };
    assert!(rows_before >= 3, "the list has at least three sessions");
    let tree_view = {
        use controllers::Controller;
        tree.view()
    };
    let mut c = BrowserController::new();
    for s in [3, 4, 5] {
        open(&mut c, s);
    }
    go(&mut c, "example.com");
    let fx = send(&mut c, BrowserMsg::Home);
    assert!(
        fx.is_empty(),
        "\"Home\" sends no commands or events: {fx:?}"
    );
    assert!(c.view().start_page);
    assert_eq!(c.view().address, "");
    {
        use controllers::Controller;
        assert_eq!(tree.view(), tree_view, "the session list is untouched");
    }
    let _ = &mut tree;
    // from the start page you can go \"forward\" to the closed page
    send(&mut c, BrowserMsg::Forward);
    assert_eq!(c.view().address, "https://example.com");
}

#[test]
fn browser_profile_is_per_session() {
    let base = Path::new("/data/agent-shell/browser");
    assert_ne!(profile_dir(base, 1), profile_dir(base, 2));
    assert_eq!(
        profile_dir(base, 7),
        Path::new("/data/agent-shell/browser/sessions/7")
    );
    assert_ne!(
        profile_dir(base, 7),
        agent_profile_dir(base, 7),
        "the agent profile is separate"
    );
    assert_eq!(
        agent_profile_dir(base, 7),
        Path::new("/data/agent-shell/browser/agent/7")
    );
    assert_ne!(agent_profile_dir(base, 7), agent_profile_dir(base, 8));

    let mut c = BrowserController::new();
    open(&mut c, 1);
    go(&mut c, "one.example.com");
    send(
        &mut c,
        BrowserMsg::Marks {
            session_id: 1,
            items: vec![link("One", "https://one.example.com/")],
            rev: 3,
        },
    );
    open(&mut c, 2);
    assert!(
        c.view().start_page,
        "the second session has its own history"
    );
    assert!(
        c.view().bookmarks.is_empty(),
        "bookmarks of the first session are not visible in the second"
    );
    go(&mut c, "two.example.com");
    open(&mut c, 1);
    let v = c.view();
    assert_eq!(
        v.address, "https://one.example.com",
        "the first session's page is in place"
    );
    assert_eq!(v.bookmarks, vec![link("One", "https://one.example.com/")]);
    assert!(!v.can_forward);
}

#[test]
fn browser_add_quick_link_writes_with_rev() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    send(
        &mut c,
        BrowserMsg::Quick {
            items: vec![link("HackerNews", "https://news.ycombinator.com/")],
            rev: 4,
        },
    );
    assert_eq!(c.view().quick.len(), 1);
    let fx = send(
        &mut c,
        BrowserMsg::AddQuick {
            title: "Example".into(),
            url: "example.org".into(),
        },
    );
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::PutQuickLinks {
            rev: 4,
            items: vec![
                link("HackerNews", "https://news.ycombinator.com/"),
                link("Example", "https://example.org")
            ],
        }],
        "address parsed by the same parse_address, write with If-Match"
    );
    assert_eq!(c.view().quick.len(), 2, "the link is visible at once");
    let dup = send(
        &mut c,
        BrowserMsg::AddQuick {
            title: "More".into(),
            url: "https://example.org".into(),
        },
    );
    assert!(dup.is_empty(), "a repeated address is not added");
    for bad in [
        ("", "example.org"),
        ("Heading", "javascript:alert(1)"),
        ("Search", "just a word"),
    ] {
        let fx = send(
            &mut c,
            BrowserMsg::AddQuick {
                title: bad.0.into(),
                url: bad.1.into(),
            },
        );
        assert!(fx.is_empty(), "{bad:?}: {fx:?}");
    }
    assert_eq!(c.view().quick.len(), 2);
    let fx = send(&mut c, BrowserMsg::Stale { session_id: None });
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::FetchQuickLinks],
        "412: re-read"
    );
}

#[test]
fn browser_bookmarks_add_remove_with_rev() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    assert!(
        commands(&send(&mut c, BrowserMsg::AddMark)).is_empty(),
        "nothing to bookmark on the start page"
    );
    go(&mut c, "example.org");
    send(
        &mut c,
        BrowserMsg::Loaded {
            session_id: 1,
            url: "https://example.org".into(),
            title: "Example".into(),
        },
    );
    send(
        &mut c,
        BrowserMsg::Marks {
            session_id: 1,
            items: vec![],
            rev: 9,
        },
    );
    let fx = send(&mut c, BrowserMsg::AddMark);
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::PutBookmarks {
            session_id: 1,
            rev: 9,
            items: vec![link("Example", "https://example.org")],
        }]
    );
    assert!(c.view().bookmarked);
    assert!(
        commands(&send(&mut c, BrowserMsg::AddMark)).is_empty(),
        "no duplicate"
    );
    let fx = send(&mut c, BrowserMsg::RemoveMark(0));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::PutBookmarks {
            session_id: 1,
            rev: 9,
            items: vec![],
        }]
    );
    assert!(!c.view().bookmarked);
    assert!(
        commands(&send(&mut c, BrowserMsg::RemoveMark(5))).is_empty(),
        "no such index"
    );
    let fx = send(
        &mut c,
        BrowserMsg::Stale {
            session_id: Some(1),
        },
    );
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::FetchBookmarks { session_id: 1 }]
    );
}

fn cmd(id: &str, url: &str) -> BrowserCommand {
    BrowserCommand {
        request_id: id.into(),
        command: "open".into(),
        url: url.into(),
    }
}

#[test]
fn browser_agent_command_runs_in_agent_profile_and_answers_once() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    go(&mut c, "user.example.com");
    let user_before = c.view().address.clone();
    let fx = send(
        &mut c,
        BrowserMsg::Agent {
            session_id: 1,
            command: cmd("r1", "https://agent.example.com/page"),
        },
    );
    assert!(
        fx.is_empty(),
        "the core gets nothing until the page is loaded"
    );
    let a = c.view().agent.unwrap_or_else(|| panic!("no agent load"));
    assert_eq!(
        (a.request_id.as_str(), a.url.as_str()),
        ("r1", "https://agent.example.com/page")
    );
    assert_eq!(
        c.view().address,
        user_before,
        "the user's page is untouched"
    );
    // redelivery of the frame does not run the command a second time
    let result = ToolResult::page(
        "r1",
        "https://agent.example.com/page",
        200,
        "<html></html>",
        40,
    );
    let fx = send(
        &mut c,
        BrowserMsg::AgentDone {
            session_id: 1,
            result: result.clone(),
        },
    );
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::PostToolResult {
            session_id: 1,
            result: result.clone()
        }]
    );
    assert!(c.view().agent.is_none());
    let again = send(
        &mut c,
        BrowserMsg::Agent {
            session_id: 1,
            command: cmd("r1", "https://agent.example.com/page"),
        },
    );
    assert!(
        again.is_empty() && c.view().agent.is_none(),
        "a repeated frame is ignored"
    );
    let dup = send(
        &mut c,
        BrowserMsg::AgentDone {
            session_id: 1,
            result,
        },
    );
    assert!(
        dup.is_empty(),
        "a second result for the same request_id is not sent"
    );
}

#[test]
fn browser_agent_bad_commands_get_error_results_without_loading() {
    let mut c = BrowserController::new();
    open(&mut c, 1);
    let fx = send(
        &mut c,
        BrowserMsg::Agent {
            session_id: 1,
            command: cmd("r2", "file:///etc/passwd"),
        },
    );
    let cmds = commands(&fx);
    assert_eq!(cmds.len(), 1, "{fx:?}");
    let CoreCommand::PostToolResult { result, .. } = cmds[0] else {
        panic!("{fx:?}")
    };
    assert_eq!(
        (result.request_id.as_str(), result.status.as_str()),
        ("r2", "denied")
    );
    assert!(c.view().agent.is_none(), "a foreign scheme is not loaded");
    let odd = BrowserCommand {
        request_id: "r3".into(),
        command: "reboot".into(),
        url: "https://x.y/".into(),
    };
    let fx = send(
        &mut c,
        BrowserMsg::Agent {
            session_id: 1,
            command: odd,
        },
    );
    let CoreCommand::PostToolResult { result, .. } = commands(&fx)[0] else {
        panic!("{fx:?}")
    };
    assert_eq!(result.status, "error");
    assert!(c.view().agent.is_none());
}

#[test]
fn tool_result_output_is_cut_at_limit_on_char_boundary() {
    let text = "é".repeat(200_000);
    let r = ToolResult::page("r", "https://x.y/", 200, &text, 1);
    let out = r.output.clone().unwrap_or_default();
    assert!(
        out.len() <= 256 * 1024 && r.truncated,
        "{} bytes",
        out.len()
    );
    assert_eq!(r.output_bytes, Some(400_000));
    assert!(out.is_char_boundary(out.len()));
    let small = ToolResult::page("r", "https://x.y/", 200, "ok", 1);
    assert!(!small.truncated && small.output.as_deref() == Some("ok"));
    let j = r.json();
    assert_eq!(j["kind"], "browser");
    assert_eq!(j["truncated"], true);
}

#[test]
fn tool_result_cut_is_exactly_256_kib_and_respects_char_boundary() {
    let ascii = "a".repeat(300_000);
    let r = ToolResult::page("r", "https://x.y/", 200, &ascii, 1);
    assert_eq!(
        r.output.as_deref().map(str::len),
        Some(262_144),
        "api.md section 5.2.1: up to 256 KiB"
    );
    assert!(r.truncated);
    // a multibyte character crosses the limit: the output is cut before it, not in the middle of it
    let text = format!("a{}", "é".repeat(150_000));
    let r = ToolResult::page("r", "https://x.y/", 200, &text, 1);
    assert_eq!(r.output.as_deref().map(str::len), Some(262_143));
    assert_eq!(r.output_bytes, Some(300_001));
    // exactly at the limit: no truncation
    let exact = "a".repeat(262_144);
    let r = ToolResult::page("r", "https://x.y/", 200, &exact, 1);
    assert!(!r.truncated && r.output.as_deref().map(str::len) == Some(262_144));
}

#[test]
fn browser_command_request_id_length_is_bounded() {
    use controllers::tool_types::parse_browser_command;
    let with =
        |id: &str| serde_json::json!({"request_id": id, "command": "open", "url": "https://x.y/"});
    assert!(
        parse_browser_command(&with("")).is_none(),
        "empty request_id"
    );
    assert!(parse_browser_command(&with(&"r".repeat(200))).is_some());
    assert!(parse_browser_command(&with(&"r".repeat(201))).is_none());
    assert!(
        parse_browser_command(&serde_json::json!({"request_id": "r"})).is_none(),
        "no command"
    );
    let no_url = parse_browser_command(&serde_json::json!({"request_id": "r", "command": "open"}));
    assert_eq!(no_url.map(|c| c.url), Some(String::new()));
}
