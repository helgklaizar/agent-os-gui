//! Tools through real controllers, the command executor and the core client against `fake-agentd`:
//! browser results, bookmarks, session settings, tasks, the agent terminal, `shell` permissions.

mod common;
mod tools_common;

use std::time::Instant;

use controllers::browser_view::BrowserMsg;
use controllers::ss_view::{NotifMode, SsMsg};
use controllers::tool_types::{BrowserCommand, ToolResult};
use controllers::{Controller, CoreCommand, Effect};
use core_client::CoreEvent;
use core_client::api::FrameKind;
use fake_agentd::Config;
use serde_json::json;
use tools_common::*;

fn now() -> Instant {
    Instant::now()
}

fn agent_status(r: &Rig, status: &str) -> bool {
    r.t.terminal.view().agent.iter().any(|x| x.status == status)
}

async fn ask(r: &mut Rig, text: &str) {
    let cid = uuid::Uuid::now_v7().to_string();
    let sent = r.http.send_message(r.session, &cid, text, &[]).await;
    assert!(sent.is_ok(), "send {text}: {sent:?}");
}

async fn pending_id(r: &Rig) -> i64 {
    for _ in 0..500 {
        if let Some(p) = r.d.pending_requests().first() {
            return p["id"].as_i64().unwrap_or(-1);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the permission request did not appear");
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_browser_roundtrip() {
    let mut r = rig().await;
    let rid = r.d.browser_command(r.session, "https://example.org/page");
    r.until("agent command to the browser", |r| {
        r.t.browser.view().agent.is_some()
    })
    .await;
    let a =
        r.t.browser
            .view()
            .agent
            .unwrap_or_else(|| panic!("no command"));
    assert_eq!(
        (a.request_id.as_str(), a.url.as_str()),
        (rid.as_str(), "https://example.org/page")
    );
    let result = ToolResult::page(&rid, "https://example.org/page", 200, "<html>ok</html>", 5);
    let fx = r.t.browser.update(
        BrowserMsg::AgentDone {
            session_id: r.session,
            result: result.clone(),
        },
        now(),
    );
    r.run(fx).await;
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    // repeating the same result is idempotent (after a connection loss the client sends it again)
    let again = r.http.post_tool_result(r.session, result.json()).await;
    assert!(again.is_ok(), "repeat: {again:?}");
    assert_eq!(again.unwrap_or_default()["status"], "accepted");
    // another result for the same request_id is rejected
    let mut other = result.clone();
    other.output = Some("other".into());
    let conflict = r.http.post_tool_result(r.session, other.json()).await;
    assert_eq!(
        conflict.err().and_then(|e| e.api().map(|a| a.status)),
        Some(409)
    );
    // redelivery of the command frame does not run it again
    let fx = r.t.browser.update(
        BrowserMsg::Agent {
            session_id: r.session,
            command: BrowserCommand {
                request_id: rid.clone(),
                command: "open".into(),
                url: "https://example.org/page".into(),
            },
        },
        now(),
    );
    assert!(fx.is_empty() && r.t.browser.view().agent.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_bookmarks_roundtrip_and_stale_revision() {
    let mut r = rig().await;
    r.until("bookmarks read", |r| {
        r.t.browser.view().session == Some(r.session)
    })
    .await;
    let fx =
        r.t.browser
            .update(BrowserMsg::Submit("example.org".into()), now());
    r.run(fx).await;
    r.t.browser.update(
        BrowserMsg::Loaded {
            session_id: r.session,
            url: "https://example.org".into(),
            title: "Example".into(),
        },
        now(),
    );
    let fx = r.t.browser.update(BrowserMsg::AddMark, now());
    r.run(fx).await;
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    let saved = r
        .http
        .bookmarks(r.session)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(texts(&json!(saved.items), "title"), vec!["Example"]);
    assert!(saved.rev >= 1);
    // another client changes the list: our write with a stale rev gets 412 and re-reads
    let theirs = vec![json!({"title": "Other", "url": "https://other.example/"})];
    assert!(
        r.http
            .put_bookmarks(r.session, saved.rev, &theirs)
            .await
            .is_ok()
    );
    let fx = r.t.browser.update(BrowserMsg::RemoveMark(0), now());
    r.run(fx).await;
    let after = r
        .http
        .bookmarks(r.session)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        texts(&json!(after.items), "title"),
        vec!["Other"],
        "the foreign entry is not overwritten"
    );
    assert_eq!(
        r.t.browser
            .view()
            .bookmarks
            .iter()
            .map(|l| l.title.clone())
            .collect::<Vec<_>>(),
        vec!["Other"],
        "the controller re-read the list after 412"
    );
    assert!(
        r.failures.is_empty(),
        "412 is not an error but a re-read: {:?}",
        r.failures
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_link_add_persists_in_core() {
    let mut r = rig().await;
    r.until("quick links", |r| r.t.browser.view().quick.len() == 5)
        .await;
    let fx = r.t.browser.update(
        BrowserMsg::AddQuick {
            title: "Example".into(),
            url: "example.org".into(),
        },
        now(),
    );
    r.run(fx).await;
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    let links = r.http.quicklinks().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(links.items.len(), 6);
    assert_eq!(links.items[5]["url"], "https://example.org");
    assert_eq!(r.t.browser.view().quick.len(), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_settings_persist_across_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let mut r = rig_with(
        Config::temp()
            .no_ticker()
            .no_event_fixtures()
            .db(db.clone()),
    )
    .await;
    for m in [
        SsMsg::SetMem(true),
        SsMsg::SetAuto(true),
        SsMsg::SetNotif(NotifMode::Important),
        SsMsg::SetPath("/work/persist".into()),
    ] {
        let fx = r.t.settings.update(m, now());
        r.run(fx).await;
    }
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    let session = r.session;
    drop(r);
    let mut again = rig_with(Config::temp().no_ticker().no_event_fixtures().db(db)).await;
    // a second session of the same name was created by the stand; read the first one's settings by its number
    again.attach(session).await;
    let fx = again.t.open(Some(session), now());
    again.run(fx).await;
    let v = again.t.settings.view();
    assert_eq!((v.mem, v.auto, v.notif), (true, true, NotifMode::Important));
    assert_eq!(v.path, "/work/persist");
}

#[tokio::test(flavor = "multi_thread")]
async fn task_create_run_emits_event_and_shows_runs() {
    let mut r = rig().await;
    let fx = r.t.settings.update(
        SsMsg::TaskCreate {
            kind: "sched".into(),
            title: "Report".into(),
            spec: "collect".into(),
            cron: Some("0 9 * * *".into()),
        },
        now(),
    );
    r.run(fx).await;
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    let tasks = r.t.settings.view().tasks;
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].schedule.contains("Asia/Jerusalem"));
    let id = tasks[0].id;
    let fx = r.t.settings.update(SsMsg::TaskRun(id), now());
    r.run(fx).await;
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    let v = r.t.settings.view();
    assert_eq!(v.runs_of, Some(id));
    assert_eq!(v.runs.len(), 1, "recent runs are shown");
    assert_eq!(v.runs[0].status, "ok");
    let events = r
        .http
        .list_events(&core_client::events::EventQuery {
            kind: Some("task".into()),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let subs = texts(&events["items"], "sub");
    assert!(
        subs.contains(&"sched".to_string()) && subs.contains(&"done".to_string()),
        "{subs:?}"
    );
    let fx = r.t.settings.update(SsMsg::TaskPause(id), now());
    r.run(fx).await;
    assert_eq!(r.rows("SELECT state FROM tasks").await, vec!["paused"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_perm_loosen_goes_403_and_is_explained() {
    let mut r = rig().await;
    assert!(
        r.http
            .put_permission("global", 0, "shell", "deny")
            .await
            .is_ok()
    );
    let fx = r.t.settings.update(
        SsMsg::Perms(controllers::permissions::PermTable::default()),
        now(),
    );
    r.run(fx).await;
    // bypassing the view: a weakening command goes straight to the core; the core answers 403 and writes a finding
    let loosen = CoreCommand::PutSessionPerm {
        session_id: r.session,
        action: "shell".into(),
        mode: controllers::permissions::Mode::Allow,
    };
    r.run(vec![Effect::Command(loosen)]).await;
    assert!(
        r.failures.is_empty(),
        "403 parsed as a refusal, not a failure: {:?}",
        r.failures
    );
    assert!(
        r.t.settings.view().notice.is_some(),
        "the reason is shown to the user"
    );
    assert_eq!(
        r.rows("SELECT kind FROM observer_findings WHERE kind = 'permission_loosen'")
            .await
            .len(),
        1,
        "finding written"
    );
    let rows = r
        .rows("SELECT mode FROM permissions WHERE scope = 'session'")
        .await;
    assert!(
        rows.is_empty(),
        "the session rule was not created: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dangerous_command_blocked_until_approved() {
    let mut r = rig().await;
    ask(&mut r, "/danger rm -rf /tmp/x").await;
    let id = pending_id(&r).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    while let Ok(ev) = r.rx.try_recv() {
        let fx = r.t.route(&ev, now());
        r.run(fx).await;
    }
    assert!(
        r.t.terminal.view().agent.is_empty(),
        "before the decision the core ran nothing"
    );
    assert!(r.http.decide_permission(id, "allow", None).await.is_ok());
    r.until("executed after 'Allow'", |r| agent_status(r, "exited"))
        .await;
    let run = r.t.terminal.view().agent[0].clone();
    assert_eq!(run.command, "rm -rf /tmp/x");
    assert!(run.output.contains("rm -rf"), "{:?}", run.output);
}

#[tokio::test(flavor = "multi_thread")]
async fn dangerous_command_rejected_does_not_run() {
    let mut r = rig().await;
    ask(&mut r, "/danger git push --force").await;
    let id = pending_id(&r).await;
    assert!(r.http.decide_permission(id, "deny", None).await.is_ok());
    r.until("denied", |r| agent_status(r, "denied")).await;
    assert!(!agent_status(&r, "started") && !agent_status(&r, "exited"));
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_decision_first_wins() {
    let mut r = rig().await;
    assert!(
        r.http
            .put_permission("global", 0, "shell", "ask")
            .await
            .is_ok()
    );
    ask(&mut r, "/ask shell ls").await;
    let id = pending_id(&r).await;
    assert!(r.http.decide_permission(id, "allow", None).await.is_ok());
    let second = r.http.decide_permission(id, "deny", None).await;
    assert_eq!(
        second.err().and_then(|e| e.api().map(|a| a.status)),
        Some(409),
        "second answer is 409"
    );
    r.until("executed once", |r| agent_status(r, "exited"))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    while let Ok(ev) = r.rx.try_recv() {
        let fx = r.t.route(&ev, now());
        r.run(fx).await;
    }
    assert_eq!(
        r.t.terminal.view().agent.len(),
        1,
        "the command ran once, the refusal did not go through"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_permission_ask_roundtrip() {
    let mut r = rig().await;
    // at the session level `shell` asks (tightening the default is allowed)
    assert!(
        r.http
            .put_permission("session", r.session, "shell", "ask")
            .await
            .is_ok()
    );
    ask(&mut r, "/ask shell ls").await;
    let id = pending_id(&r).await;
    // "Always" at the session level is not allowed, at the global one it creates a rule
    let bad = r
        .http
        .decide_permission(
            id,
            "always",
            Some(json!({"scope": "session", "scope_id": r.session, "mode": "allow"})),
        )
        .await;
    assert_eq!(bad.err().and_then(|e| e.api().map(|a| a.status)), Some(422));
    let ok = r
        .http
        .decide_permission(
            id,
            "always",
            Some(json!({"scope": "global", "scope_id": 0, "mode": "allow"})),
        )
        .await;
    assert!(ok.is_ok(), "{ok:?}");
    r.until("first execution", |r| agent_status(r, "exited"))
        .await;
    assert_eq!(
        r.rows("SELECT mode FROM permissions WHERE scope = 'global' AND action = 'shell'")
            .await,
        vec!["allow"]
    );
    assert_eq!(
        r.rows("SELECT mode FROM permissions WHERE scope = 'session'")
            .await,
        vec!["ask"],
        "session level not weakened"
    );
    // a session with an `ask` rule asks again on the next request: the strict one wins
    ask(&mut r, "/ask shell pwd").await;
    let second = pending_id(&r).await;
    assert_ne!(second, id);
    assert!(
        r.http
            .decide_permission(second, "allow", None)
            .await
            .is_ok(),
        "'Allow' runs once"
    );
    r.until("second execution", |r| {
        r.t.terminal
            .view()
            .agent
            .iter()
            .any(|x| x.command == "pwd" && x.status == "exited")
    })
    .await;
    assert!(r.d.pending_requests().is_empty());
    // a session without its own rule follows the global one: no card
    let other = r
        .http
        .create_session("Another", None, None)
        .await
        .unwrap_or_default()["id"]
        .as_i64()
        .unwrap_or(-1);
    let cid = uuid::Uuid::now_v7().to_string();
    assert!(
        r.http
            .send_message(other, &cid, "/ask shell date", &[])
            .await
            .is_ok()
    );
    for _ in 0..200 {
        if !r
            .rows("SELECT decision FROM actions_log WHERE target = 'date'")
            .await
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        r.rows("SELECT decision FROM actions_log WHERE target = 'date'")
            .await,
        vec!["allow"],
        "by the rule, without a question"
    );
    assert!(r.d.pending_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_unavailable_blocks_dangerous() {
    let mut r = rig().await;
    r.d.set_observer_available(false);
    ask(&mut r, "/danger rm -rf /").await;
    r.until("fail-closed", |r| agent_status(r, "denied")).await;
    assert!(!agent_status(&r, "started"));
    assert!(
        r.d.pending_requests().is_empty(),
        "the request closed at once"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_output_panel_shows_pty_output() {
    let mut r = rig().await;
    assert!(
        r.http
            .put_permission("global", 0, "shell", "allow")
            .await
            .is_ok()
    );
    ask(&mut r, "/ask shell echo hello").await;
    r.until("agent output", |r| agent_status(r, "exited")).await;
    let run = r.t.terminal.view().agent[0].clone();
    assert_eq!(run.command, "echo hello");
    assert!(run.output.contains("done"), "{:?}", run.output);
    assert_eq!(run.exit_code, Some(0));
    assert!(r.failures.is_empty());
    // terminal output does not go to `tool-results`: the core knows such a request_id as a terminal one and refuses by kind
    let wrong = r
        .http
        .post_tool_result(
            r.session,
            json!({"request_id": run.request_id, "kind": "browser", "status": "ok"}),
        )
        .await;
    assert_eq!(
        wrong.err().and_then(|e| e.api().map(|a| a.status)),
        Some(422)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_command_runs_in_kernel_pty_not_gui() {
    let mut r = rig().await;
    assert!(
        r.http
            .put_permission("global", 0, "shell", "allow")
            .await
            .is_ok()
    );
    ask(&mut r, "/ask shell touch /tmp/agent-os-s6-must-not-exist").await;
    let mut seen_effects = 0;
    let end = Instant::now() + common::WAIT;
    while !agent_status(&r, "exited") {
        let left = end.saturating_duration_since(Instant::now());
        let Ok(Ok(ev)) = tokio::time::timeout(left, r.rx.recv()).await else {
            panic!("no agent output")
        };
        if let CoreEvent::Frame(f) = &ev
            && f.kind == FrameKind::Event
            && f.data["kind"]
                .as_str()
                .is_some_and(|k| k.starts_with("terminal."))
        {
            let fx = r.t.route(&ev, now());
            seen_effects += fx.len();
            r.run(fx).await;
        } else {
            let fx = r.t.route(&ev, now());
            r.run(fx).await;
        }
    }
    assert_eq!(
        seen_effects, 0,
        "agent frames of the terminal produce neither commands nor processes in the GUI"
    );
    assert!(
        !std::path::Path::new("/tmp/agent-os-s6-must-not-exist").exists(),
        "the GUI does not run agent commands"
    );
    let user = r.t.terminal.view().user;
    assert!(
        user.as_ref().is_none_or(|u| !u.running),
        "the user VTE did not start and received no commands"
    );
}
