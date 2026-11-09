//! S6-12: hostile input in tools: page address and title, bookmark names, session name and path, task names,
//! agent commands and output. Expectation: text verbatim (no control chars, no markup), length capped, code not executed.

mod tree_common;

use std::path::PathBuf;
use std::time::Instant;

use controllers::browser_view::BrowserMsg;
use controllers::controllers::browser::BrowserController;
use controllers::controllers::session_settings::SessionSettingsController;
use controllers::controllers::terminal::TerminalController;
use controllers::ss_view::SsMsg;
use controllers::terminal_view::TermMsg;
use controllers::testkit::evil_strings;
use controllers::tool_types::{TaskItem, TermFrame};
use controllers::{Controller, CoreCommand, Effect};
use tree_common::session;

fn is_plain(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

#[test]
fn browser_address_is_escaped() {
    for evil in evil_strings() {
        let mut c = BrowserController::new();
        c.update(BrowserMsg::Session(Some(1)), Instant::now());
        let fx = c.update(BrowserMsg::Submit(evil.clone()), Instant::now());
        assert!(fx.is_empty());
        let v = c.view();
        assert!(
            is_plain(&v.address),
            "{evil:.30}: control chars in the address"
        );
        assert!(v.address.chars().count() <= 2049, "the address is capped");
        if let Some(load) = v.load {
            assert!(
                load.url.starts_with("https://") || load.url.starts_with("http://"),
                "only http and https are loaded: {:.60}",
                load.url
            );
            assert!(
                !load.url.contains(['<', '>', '"', ' ']),
                "no markup in the load address: {:.60}",
                load.url
            );
            assert!(
                load.url.len() <= 12_300,
                "load address length is capped: {}",
                load.url.len()
            );
        }
        c.update(
            BrowserMsg::Loaded {
                session_id: 1,
                url: "https://x.example/".into(),
                title: evil.clone(),
            },
            Instant::now(),
        );
        let v = c.view();
        assert!(is_plain(&v.title), "the title has no control chars");
        assert!(v.title.chars().count() <= 201);
        if evil.chars().count() < 100 && is_plain(&evil) {
            assert_eq!(
                v.title, evil,
                "a short harmless title verbatim, markup not parsed"
            );
        }
    }
}

#[test]
fn browser_quick_link_titles_are_literal_or_refused() {
    for evil in evil_strings() {
        let mut c = BrowserController::new();
        c.update(BrowserMsg::Session(Some(1)), Instant::now());
        let fx = c.update(
            BrowserMsg::AddQuick {
                title: evil.clone(),
                url: "https://example.org/".into(),
            },
            Instant::now(),
        );
        for e in fx {
            let Effect::Command(CoreCommand::PutQuickLinks { items, .. }) = e else {
                continue;
            };
            assert!(
                items
                    .iter()
                    .all(|l| is_plain(&l.title) && l.title.chars().count() <= 201),
                "{evil:.30}"
            );
        }
    }
}

#[test]
fn session_settings_text_is_escaped() {
    for evil in evil_strings() {
        let mut c = SessionSettingsController::new();
        let mut s = session(1, None, &evil);
        s.path = Some(evil.clone());
        c.update(SsMsg::Snapshot(vec![s]), Instant::now());
        c.update(SsMsg::Open(Some(1)), Instant::now());
        let v = c.view();
        assert_eq!(v.name, evil, "session name verbatim");
        assert_eq!(v.path, evil, "path verbatim");
        c.update(SsMsg::AskDelete, Instant::now());
        let lines = c.view().confirm.map(|c| c.lines).unwrap_or_default();
        assert!(
            lines.iter().any(|l| l.contains(&evil)),
            "name in the confirmation list verbatim"
        );
        let task = TaskItem {
            id: 1,
            kind: "deleg".into(),
            title: evil.clone(),
            cron: None,
            tz: "Asia/Jerusalem".into(),
            state: "active".into(),
        };
        c.update(SsMsg::TaskChanged(task), Instant::now());
        assert_eq!(c.view().tasks[0].title, evil, "task title verbatim");
        let fx = c.update(
            SsMsg::TaskCreate {
                kind: "deleg".into(),
                title: evil.clone(),
                spec: String::new(),
                cron: None,
            },
            Instant::now(),
        );
        // the request goes to the core as data (JSON), not as markup or a command
        assert!(fx.len() <= 1);
    }
}

#[test]
fn agent_terminal_text_is_kept_literally_and_bounded() {
    for evil in evil_strings() {
        let mut c = TerminalController::new(PathBuf::from("/home/u"));
        c.update(TermMsg::Session(Some(1)), Instant::now());
        c.update(
            TermMsg::Agent {
                session_id: 1,
                frame: TermFrame::Command {
                    request_id: "r".into(),
                    command: evil.clone(),
                    cwd: evil.clone(),
                },
            },
            Instant::now(),
        );
        c.update(
            TermMsg::Agent {
                session_id: 1,
                frame: TermFrame::Output {
                    request_id: "r".into(),
                    seq: 0,
                    stderr: false,
                    data: evil.clone(),
                },
            },
            Instant::now(),
        );
        let run = &c.view().agent[0];
        assert_eq!(run.command, evil);
        assert_eq!(
            run.output, evil,
            "output verbatim, shown by the view without markup"
        );
        assert!(run.output.len() <= 256 * 1024);
    }
}
