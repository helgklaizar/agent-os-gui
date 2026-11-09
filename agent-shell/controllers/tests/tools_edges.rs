//! S6-12: edge cases of the tools controllers found by mutation testing (`cargo mutants --in-diff`): agent output at the char
//! boundary, late and repeated frames, deleting a parent with a thread, "forward" on the last page, the command memory limit.

mod tree_common;

use std::path::PathBuf;
use std::time::Instant;

use controllers::browser_view::BrowserMsg;
use controllers::controllers::browser::BrowserController;
use controllers::controllers::session_settings::SessionSettingsController;
use controllers::controllers::terminal::TerminalController;
use controllers::permissions::PermTable;
use controllers::ss_view::SsMsg;
use controllers::terminal_view::TermMsg;
use controllers::tool_types::{BrowserCommand, TermFrame, parse_term_frame};
use controllers::tree_types::TreeFrame;
use controllers::{Controller, CoreCommand, Effect};
use serde_json::json;
use tree_common::{session, thread};

fn now() -> Instant {
    Instant::now()
}

fn term() -> TerminalController {
    TerminalController::new(PathBuf::from("/home/u"))
}

fn frame(c: &mut TerminalController, s: i64, f: TermFrame) {
    c.update(
        TermMsg::Agent {
            session_id: s,
            frame: f,
        },
        now(),
    );
}

fn command(id: &str, text: &str) -> TermFrame {
    TermFrame::Command {
        request_id: id.into(),
        command: text.into(),
        cwd: "~".into(),
    }
}

fn output(id: &str, seq: u64, data: &str) -> TermFrame {
    TermFrame::Output {
        request_id: id.into(),
        seq,
        stderr: false,
        data: data.into(),
    }
}

fn status(id: &str, st: &str) -> TermFrame {
    TermFrame::Status {
        request_id: id.into(),
        status: st.into(),
        exit_code: None,
        duration_ms: None,
    }
}

#[test]
fn terminal_wire_frames_command_and_status_parse() {
    let c = parse_term_frame(
        &json!({"kind": "terminal.command", "request_id": "r", "command": "ls", "cwd": "/w"}),
    );
    assert_eq!(c, Some(command("r", "ls").with_cwd("/w")));
    let no_cwd =
        parse_term_frame(&json!({"kind": "terminal.command", "request_id": "r", "command": "ls"}));
    assert_eq!(no_cwd, Some(command("r", "ls").with_cwd("")));
    let s = parse_term_frame(
        &json!({"kind": "terminal.status", "request_id": "r", "status": "exited", "exit_code": 2, "duration_ms": 15}),
    );
    assert_eq!(
        s,
        Some(TermFrame::Status {
            request_id: "r".into(),
            status: "exited".into(),
            exit_code: Some(2),
            duration_ms: Some(15)
        })
    );
    assert!(parse_term_frame(&json!({"kind": "terminal.command", "request_id": "r"})).is_none());
}

trait WithCwd {
    fn with_cwd(self, cwd: &str) -> Self;
}

impl WithCwd for TermFrame {
    fn with_cwd(self, new: &str) -> TermFrame {
        match self {
            TermFrame::Command {
                request_id,
                command,
                ..
            } => TermFrame::Command {
                request_id,
                command,
                cwd: new.into(),
            },
            other => other,
        }
    }
}

#[test]
fn terminal_repeated_command_frame_keeps_first_text_and_late_command_fills_empty() {
    let mut c = term();
    c.update(TermMsg::Session(Some(1)), now());
    frame(&mut c, 1, command("r1", "ls"));
    frame(&mut c, 1, command("r1", "rm -rf /"));
    assert_eq!(
        c.view().agent[0].command,
        "ls",
        "a repeated frame does not replace the command"
    );
    // output arrived before the command (the subscription caught up mid-stream): the entry is created empty, the command fills it
    frame(&mut c, 1, output("r9", 0, "tail"));
    assert_eq!(c.view().agent[1].command, "");
    frame(&mut c, 1, command("r9", "late"));
    assert_eq!(c.view().agent[1].command, "late");
    assert_eq!(c.view().agent[1].cwd, "~");
    assert_eq!(c.view().agent[1].output, "tail");
}

#[test]
fn terminal_status_never_goes_back_from_final() {
    let mut c = term();
    c.update(TermMsg::Session(Some(1)), now());
    frame(&mut c, 1, command("r", "ls"));
    frame(&mut c, 1, status("r", "started"));
    assert_eq!(c.view().agent[0].status, "started");
    frame(&mut c, 1, status("r", "exited"));
    frame(&mut c, 1, status("r", "started"));
    assert_eq!(
        c.view().agent[0].status,
        "exited",
        "a late started does not cancel the final result"
    );
    frame(&mut c, 1, status("r", "killed"));
    assert_eq!(
        c.view().agent[0].status,
        "killed",
        "a final result is replaced by another final result"
    );
}

#[test]
fn terminal_output_cut_lands_before_a_multibyte_char() {
    let mut c = term();
    c.update(TermMsg::Session(Some(1)), now());
    frame(&mut c, 1, command("r", "yes"));
    frame(&mut c, 1, output("r", 0, "a"));
    frame(&mut c, 1, output("r", 1, &"é".repeat(140_000)));
    let run = &c.view().agent[0];
    assert!(run.truncated);
    assert_eq!(
        run.output.len(),
        262_143,
        "the 262144 limit falls inside a char: cut before it"
    );
    assert!(run.output.is_char_boundary(run.output.len()));
    frame(&mut c, 1, output("r", 2, "more"));
    assert_eq!(
        c.view().agent[0].output.len(),
        262_143,
        "output does not grow past the limit"
    );
}

#[test]
fn terminal_session_delete_takes_threads_with_it_and_keeps_others() {
    let mut c = term();
    let mut t2 = thread(2, 1, None, "thread");
    t2.path = None;
    c.update(
        TermMsg::Snapshot(vec![
            session(1, None, "parent"),
            t2,
            session(3, None, "other"),
        ]),
        now(),
    );
    for id in [1, 2, 3] {
        c.update(TermMsg::Session(Some(id)), now());
        c.update(TermMsg::Spawned(id), now());
        frame(&mut c, id, command("r", "ls"));
    }
    assert_eq!(c.view().live, vec![1, 2, 3]);
    c.update(TermMsg::Tree(TreeFrame::SessionDeleted(1)), now());
    assert_eq!(
        c.view().live,
        vec![3],
        "the parent and its thread are gone, the other session remains"
    );
    c.update(TermMsg::Session(Some(2)), now());
    assert!(
        c.view().agent.is_empty(),
        "agent output of the deleted thread is forgotten"
    );
    c.update(TermMsg::Session(Some(3)), now());
    assert_eq!(c.view().agent.len(), 1);
}

fn browser_at(entries: &[&str]) -> BrowserController {
    let mut c = BrowserController::new();
    c.update(BrowserMsg::Session(Some(1)), now());
    for e in entries {
        c.update(BrowserMsg::Submit((*e).into()), now());
    }
    c
}

#[test]
fn browser_forward_stops_at_the_last_page() {
    let mut c = browser_at(&["a.example.com", "b.example.com", "c.example.com"]);
    let before = c.view();
    c.update(BrowserMsg::Forward, now());
    assert_eq!(
        c.view(),
        before,
        "on the last page \"forward\" does nothing"
    );
    c.update(BrowserMsg::Back, now());
    c.update(BrowserMsg::Back, now());
    assert_eq!(c.view().address, "https://a.example.com");
    c.update(BrowserMsg::Forward, now());
    assert_eq!(c.view().address, "https://b.example.com");
    c.update(BrowserMsg::Forward, now());
    assert_eq!(c.view().address, "https://c.example.com");
    let seq = c.view().load.map_or(0, |l| l.seq);
    c.update(BrowserMsg::Forward, now());
    assert_eq!(c.view().load.map_or(0, |l| l.seq), seq, "no new load");
    assert!(!c.view().can_forward);
}

#[test]
fn browser_link_after_back_replaces_the_forward_tail() {
    let mut c = browser_at(&["a.example.com", "b.example.com", "c.example.com"]);
    c.update(BrowserMsg::Back, now());
    c.update(BrowserMsg::Back, now());
    c.update(
        BrowserMsg::Loaded {
            session_id: 1,
            url: "https://x.example.com/".into(),
            title: "X".into(),
        },
        now(),
    );
    assert_eq!(c.view().address, "https://x.example.com/");
    assert!(!c.view().can_forward, "the tail b, c is dropped");
    c.update(BrowserMsg::Back, now());
    assert_eq!(
        c.view().address,
        "https://a.example.com",
        "before x there was a, not empty"
    );
}

fn agent_cmd(id: &str, url: &str) -> BrowserMsg {
    BrowserMsg::Agent {
        session_id: 1,
        command: BrowserCommand {
            request_id: id.into(),
            command: "open".into(),
            url: url.into(),
        },
    }
}

#[test]
fn browser_agent_requests_are_remembered_and_the_memory_is_bounded() {
    let mut c = BrowserController::new();
    c.update(BrowserMsg::Session(Some(1)), now());
    c.update(agent_cmd("r1", "https://one.example/"), now());
    c.update(agent_cmd("r2", "https://two.example/"), now());
    c.update(agent_cmd("r3", "https://three.example/"), now());
    assert_eq!(c.view().agent.map(|a| a.request_id), Some("r3".into()));
    let fx = c.update(agent_cmd("r1", "https://again.example/"), now());
    assert!(fx.is_empty());
    assert_eq!(
        c.view().agent.map(|a| a.url),
        Some("https://three.example/".into()),
        "a repeated old request_id is ignored"
    );
    // memory limit: the last 256 requests
    let mut d = BrowserController::new();
    d.update(BrowserMsg::Session(Some(1)), now());
    for n in 0..300 {
        d.update(agent_cmd(&format!("q{n}"), "file:///x"), now());
    }
    let recent = d.update(agent_cmd("q299", "file:///x"), now());
    assert!(recent.is_empty(), "the last request is remembered");
    let oldest = d.update(agent_cmd("q0", "file:///x"), now());
    assert_eq!(
        oldest.len(),
        1,
        "the oldest is forgotten: the reply goes out again"
    );
}

fn ss_with_threads() -> SessionSettingsController {
    let mut c = SessionSettingsController::new();
    c.update(
        SsMsg::Snapshot(vec![
            session(7, None, "Parent"),
            thread(8, 7, None, "Thread"),
            session(9, None, "Other"),
        ]),
        now(),
    );
    c
}

#[test]
fn session_settings_delete_of_parent_closes_its_thread_and_keeps_others() {
    let mut c = ss_with_threads();
    c.update(SsMsg::Open(Some(8)), now());
    c.update(SsMsg::Tree(TreeFrame::SessionDeleted(7)), now());
    assert_eq!(
        c.view().session_id,
        None,
        "the thread of the deleted parent is closed"
    );
    let mut c = ss_with_threads();
    c.update(SsMsg::Open(Some(9)), now());
    c.update(SsMsg::Tree(TreeFrame::SessionDeleted(7)), now());
    assert_eq!(
        c.view().session_id,
        Some(9),
        "the other session is untouched"
    );
    assert_eq!(c.view().name, "Other");
    c.update(SsMsg::Tree(TreeFrame::SessionDeleted(9)), now());
    assert_eq!(c.view().session_id, None);
}

#[test]
fn session_settings_perm_fields_have_distinct_labels_and_no_note_without_rules() {
    let mut c = ss_with_threads();
    c.update(SsMsg::Open(Some(7)), now());
    c.update(SsMsg::Perms(PermTable::default()), now());
    let fields = c.view().perms;
    let labels: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["Sending messages", "Deletion", "Shell commands"]
    );
    for f in &fields {
        assert!(f.options.iter().all(|o| o.enabled), "{}", f.action);
        assert!(
            f.note.is_none(),
            "no explanation without higher rules: {}",
            f.action
        );
        assert!(!f.locked);
    }
}

#[test]
fn session_settings_delegate_task_without_cron_is_created() {
    let mut c = ss_with_threads();
    c.update(SsMsg::Open(Some(7)), now());
    let fx = c.update(
        SsMsg::TaskCreate {
            kind: "deleg".into(),
            title: "Sort the mail".into(),
            spec: "reply".into(),
            cron: None,
        },
        now(),
    );
    let cmds: Vec<&CoreCommand> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(
        cmds,
        vec![&CoreCommand::CreateTask {
            session_id: 7,
            kind: "deleg".into(),
            title: "Sort the mail".into(),
            spec: "reply".into(),
            cron: None
        }]
    );
    assert!(c.view().notice.is_none());
}

#[test]
fn browser_can_go_forward_from_the_start_page_after_home_or_back() {
    let mut c = browser_at(&["a.example.com"]);
    c.update(BrowserMsg::Home, now());
    assert!(
        c.view().start_page && c.view().can_forward,
        "after \"Home\" one can go forward again"
    );
    assert!(!c.view().can_back);
    let mut d = browser_at(&["a.example.com"]);
    d.update(BrowserMsg::Back, now());
    assert!(
        d.view().start_page && d.view().can_forward,
        "back from the first page goes to the start page, forward is available"
    );
    let fresh = browser_at(&[]);
    assert!(
        !fresh.view().can_forward && !fresh.view().can_back,
        "a clean session has nowhere to go"
    );
}

#[test]
fn session_settings_task_run_and_runs_of_unknown_task_do_nothing() {
    let mut c = ss_with_threads();
    c.update(SsMsg::Open(Some(7)), now());
    c.update(
        SsMsg::Tasks {
            session_id: 7,
            items: vec![controllers::tool_types::TaskItem {
                id: 1,
                kind: "deleg".into(),
                title: "t".into(),
                cron: None,
                tz: "Asia/Jerusalem".into(),
                state: "active".into(),
            }],
        },
        now(),
    );
    for msg in [
        SsMsg::TaskRun(404),
        SsMsg::ShowRuns(404),
        SsMsg::TaskPause(404),
        SsMsg::TaskResume(404),
    ] {
        let fx = c.update(msg, now());
        assert!(fx.is_empty(), "{fx:?}");
    }
    assert_eq!(c.view().runs_of, None);
}
