//! Session settings: values, permissions only toward tightening, confirmations, tasks.

mod tree_common;

use std::time::Instant;

use controllers::controllers::session_settings::SessionSettingsController;
use controllers::permissions::{Mode, PermTable, PermissionRow};
use controllers::ss_view::{ConfirmKind, ModelChoice, NotifMode, PERM_ACTIONS, SsMsg, SsView};
use controllers::tool_types::{RunItem, TaskItem};
use controllers::tree_types::{PersonaItem, SessionItem};
use controllers::{CfgPatch, Controller, CoreCommand, Effect};
use tree_common::{session, thread};

fn send(c: &mut SessionSettingsController, m: SsMsg) -> Vec<Effect> {
    c.update(m, Instant::now())
}

fn cmds(fx: &[Effect]) -> Vec<CoreCommand> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

fn row(scope: &str, id: i64, action: &str, mode: Mode) -> PermissionRow {
    PermissionRow {
        scope: scope.into(),
        scope_id: id,
        action: action.into(),
        mode,
    }
}

fn loaded() -> SessionSettingsController {
    let mut c = SessionSettingsController::new();
    let mut s = session(7, Some(2), "Release");
    s.persona_id = Some(10);
    s.path = Some("/work/rel".into());
    let items: Vec<SessionItem> = vec![
        s,
        thread(8, 7, Some(2), "Review"),
        thread(9, 7, Some(2), "Hotfix"),
    ];
    send(&mut c, SsMsg::Snapshot(items));
    send(
        &mut c,
        SsMsg::Personas(vec![PersonaItem {
            id: 10,
            name: "Work".into(),
            color: "#000".into(),
        }]),
    );
    send(&mut c, SsMsg::Open(Some(7)));
    c
}

fn field(v: &SsView, action: &str) -> controllers::ss_view::PermField {
    v.perms
        .iter()
        .find(|f| f.action == action)
        .cloned()
        .unwrap_or_else(|| panic!("no field {action}"))
}

#[test]
fn session_settings_open_fetches_tasks_and_permissions() {
    let mut c = SessionSettingsController::new();
    let fx = send(&mut c, SsMsg::Open(Some(7)));
    let cmds = cmds(&fx);
    assert!(cmds.contains(&CoreCommand::FetchTasks { session_id: 7 }));
    assert!(cmds.contains(&CoreCommand::FetchPermissions));
    assert!(send(&mut c, SsMsg::Open(None)).is_empty());
    assert_eq!(c.view().session_id, None);
}

#[test]
fn session_settings_view_shows_session_values() {
    let c = loaded();
    let v = c.view();
    assert_eq!(v.name, "Release");
    assert_eq!(v.path, "/work/rel");
    assert_eq!(v.persona, Some(10));
    assert_eq!(v.personas, vec![(10, "Work".to_string())]);
    assert!(!v.is_thread);
    assert_eq!(
        v.perms
            .iter()
            .map(|f| f.action.as_str())
            .collect::<Vec<_>>(),
        PERM_ACTIONS.to_vec()
    );
}

#[test]
fn session_settings_changes_go_to_core_as_single_fields() {
    let mut c = loaded();
    let fx = send(&mut c, SsMsg::SetPersona(Some(10)));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::SetSessionPersona {
            session_id: 7,
            persona_id: Some(10)
        }]
    );
    let fx = send(&mut c, SsMsg::SetPath("/work/new".into()));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::SetSessionPath {
            session_id: 7,
            path: "/work/new".into()
        }]
    );
    assert_eq!(c.view().path, "/work/new");
    let fx = send(&mut c, SsMsg::SetModel(Some(2)));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::PatchSessionCfg {
            session_id: 7,
            cfg: CfgPatch {
                model: Some(Some(2)),
                ..CfgPatch::default()
            }
        }]
    );
    assert_eq!(c.view().model, Some(2));
    let fx = send(&mut c, SsMsg::SetModel(None));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::PatchSessionCfg {
            session_id: 7,
            cfg: CfgPatch {
                model: Some(None),
                ..CfgPatch::default()
            }
        }],
        "'same as persona'"
    );
    for (m, patch) in [
        (
            SsMsg::SetMem(true),
            CfgPatch {
                mem: Some(true),
                ..CfgPatch::default()
            },
        ),
        (
            SsMsg::SetAuto(true),
            CfgPatch {
                auto: Some(true),
                ..CfgPatch::default()
            },
        ),
        (
            SsMsg::SetNotif(NotifMode::Off),
            CfgPatch {
                notif: Some("off".into()),
                ..CfgPatch::default()
            },
        ),
    ] {
        let fx = send(&mut c, m);
        assert_eq!(
            cmds(&fx),
            vec![CoreCommand::PatchSessionCfg {
                session_id: 7,
                cfg: patch
            }]
        );
    }
    let v = c.view();
    assert!(v.mem && v.auto);
    assert_eq!(v.notif, NotifMode::Off);
    send(
        &mut c,
        SsMsg::Models(vec![ModelChoice {
            account_id: 2,
            label: "Claude".into(),
        }]),
    );
    assert_eq!(c.view().models.len(), 1);
}

#[test]
fn session_settings_ctx_only_for_threads() {
    let mut c = loaded();
    assert!(
        send(&mut c, SsMsg::SetCtxInherit(false)).is_empty(),
        "a session has no parent context"
    );
    send(&mut c, SsMsg::Open(Some(8)));
    assert!(c.view().is_thread);
    assert!(c.view().inherit_ctx, "inherits by default");
    let fx = send(&mut c, SsMsg::SetCtxInherit(false));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::PatchSessionCfg {
            session_id: 8,
            cfg: CfgPatch {
                ctx: Some("none".into()),
                ..CfgPatch::default()
            }
        }]
    );
    assert!(!c.view().inherit_ctx);
    let fx = send(&mut c, SsMsg::SetCtxInherit(true));
    assert_eq!(cmds(&fx).len(), 1);
}

#[test]
fn move_does_not_rename_session() {
    let mut c = loaded();
    let fx = send(&mut c, SsMsg::MoveTo(Some(3)));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::MoveSession {
            id: 7,
            folder_id: Some(3)
        }],
        "only the folder is in the command"
    );
    assert_eq!(c.view().name, "Release", "the name did not change");
    let fx = send(&mut c, SsMsg::MoveTo(None));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::MoveSession {
            id: 7,
            folder_id: None
        }]
    );
    assert_eq!(c.view().name, "Release");
}

#[test]
fn session_perms_cannot_loosen_global_deny() {
    let mut c = loaded();
    send(
        &mut c,
        SsMsg::Perms(PermTable {
            rows: vec![
                row("global", 0, "shell", Mode::Deny),
                row("persona", 10, "send", Mode::Ask),
            ],
        }),
    );
    let v = c.view();
    let shell = field(&v, "shell");
    assert!(shell.locked, "a global deny locks the field");
    assert!(
        shell
            .note
            .as_deref()
            .is_some_and(|n| !n.is_empty() && !n.contains('⟦')),
        "there is an explanation"
    );
    assert_eq!(shell.effective, Mode::Deny);
    let enabled: Vec<Mode> = shell
        .options
        .iter()
        .filter(|o| o.enabled)
        .map(|o| o.mode)
        .collect();
    assert_eq!(enabled, vec![Mode::Deny], "only deny is available");
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "shell".into(),
            mode: Some(Mode::Allow),
        },
    );
    assert!(
        cmds(&fx).is_empty(),
        "loosening does not go to the core: {fx:?}"
    );
    assert!(c.view().notice.is_some(), "the reason is shown");
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "shell".into(),
            mode: Some(Mode::Ask),
        },
    );
    assert!(cmds(&fx).is_empty());
    // persona ask: allow is unavailable, ask and deny are allowed
    let send_field = field(&c.view(), "send");
    let enabled: Vec<Mode> = send_field
        .options
        .iter()
        .filter(|o| o.enabled)
        .map(|o| o.mode)
        .collect();
    assert_eq!(enabled, vec![Mode::Ask, Mode::Deny]);
    assert!(!send_field.locked);
    assert!(send_field.note.is_some());
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "send".into(),
            mode: Some(Mode::Allow),
        },
    );
    assert!(
        cmds(&fx).is_empty(),
        "allow under a persona ask is loosening"
    );
}

#[test]
fn session_perms_tighten_allowed() {
    let mut c = loaded();
    send(
        &mut c,
        SsMsg::Perms(PermTable {
            rows: vec![row("global", 0, "send", Mode::Allow)],
        }),
    );
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "send".into(),
            mode: Some(Mode::Ask),
        },
    );
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::PutSessionPerm {
            session_id: 7,
            action: "send".into(),
            mode: Mode::Ask
        }]
    );
    assert_eq!(field(&c.view(), "send").selected, Some(Mode::Ask));
    assert_eq!(field(&c.view(), "send").effective, Mode::Ask);
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "send".into(),
            mode: Some(Mode::Deny),
        },
    );
    assert_eq!(cmds(&fx).len(), 1, "allow → deny is allowed");
    assert_eq!(field(&c.view(), "send").effective, Mode::Deny);
    // 'same as persona': the value of the higher level
    let fx = send(
        &mut c,
        SsMsg::SetPerm {
            action: "send".into(),
            mode: None,
        },
    );
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::PutSessionPerm {
            session_id: 7,
            action: "send".into(),
            mode: Mode::Allow
        }]
    );
    assert_eq!(
        field(&c.view(), "send").selected,
        None,
        "matches the higher level: 'same as persona' again"
    );
    assert!(
        cmds(&send(
            &mut c,
            SsMsg::SetPerm {
                action: "pay".into(),
                mode: Some(Mode::Deny)
            }
        ))
        .is_empty(),
        "an action not in the session list"
    );
}

#[test]
fn session_perm_rejected_by_core_refetches_and_explains() {
    let mut c = loaded();
    let fx = send(&mut c, SsMsg::PermRejected);
    assert_eq!(cmds(&fx), vec![CoreCommand::FetchPermissions]);
    assert!(c.view().notice.is_some());
}

#[test]
fn session_settings_clear_and_delete_require_confirm() {
    let mut c = loaded();
    let fx = send(&mut c, SsMsg::AskClear);
    assert!(
        cmds(&fx).is_empty(),
        "clearing without confirmation is not sent"
    );
    let confirm = c
        .view()
        .confirm
        .unwrap_or_else(|| panic!("no confirmation"));
    assert_eq!(confirm.kind, ConfirmKind::Clear);
    assert!(
        confirm.lines.len() >= 2,
        "list of what will be lost: {:?}",
        confirm.lines
    );
    assert!(confirm.lines.iter().any(|l| l.contains("Release")));
    let fx = send(&mut c, SsMsg::Cancel);
    assert!(
        cmds(&fx).is_empty() && c.view().confirm.is_none(),
        "cancel sends nothing"
    );
    assert!(
        cmds(&send(&mut c, SsMsg::Confirm)).is_empty(),
        "'Confirm' without a question does nothing"
    );

    send(&mut c, SsMsg::AskClear);
    let fx = send(&mut c, SsMsg::Confirm);
    assert_eq!(cmds(&fx), vec![CoreCommand::ClearSession { session_id: 7 }]);
    assert!(c.view().confirm.is_none());

    let fx = send(&mut c, SsMsg::AskDelete);
    assert!(cmds(&fx).is_empty());
    let confirm = c
        .view()
        .confirm
        .unwrap_or_else(|| panic!("no confirmation"));
    assert_eq!(confirm.kind, ConfirmKind::Delete);
    assert!(
        confirm.lines.iter().any(|l| l.contains('2')),
        "the list contains the thread count (2): {:?}",
        confirm.lines
    );
    let fx = send(&mut c, SsMsg::Confirm);
    assert_eq!(cmds(&fx), vec![CoreCommand::DeleteSession { id: 7 }]);
}

#[test]
fn session_settings_export_and_new_thread() {
    let mut c = loaded();
    assert_eq!(
        cmds(&send(&mut c, SsMsg::Export)),
        vec![CoreCommand::ExportSession { session_id: 7 }]
    );
    send(
        &mut c,
        SsMsg::Exported {
            path: "~/.agentos/export/session-7.md".into(),
            size: 10,
        },
    );
    assert_eq!(
        c.view().exported.as_deref(),
        Some("~/.agentos/export/session-7.md")
    );
    let fx = cmds(&send(&mut c, SsMsg::NewThread));
    assert!(
        matches!(&fx[..], [CoreCommand::CreateThread { parent_id: 7, name }] if !name.is_empty() && !name.contains('⟦')),
        "{fx:?}"
    );
    send(&mut c, SsMsg::Open(Some(8)));
    assert!(
        cmds(&send(&mut c, SsMsg::NewThread)).is_empty(),
        "a thread does not create a thread"
    );
}

fn task(id: i64, kind: &str, state: &str) -> TaskItem {
    TaskItem {
        id,
        kind: kind.into(),
        title: format!("task {id}"),
        cron: (kind == "sched").then(|| "0 9 * * *".into()),
        tz: "Asia/Jerusalem".into(),
        state: state.into(),
    }
}

#[test]
fn session_settings_tasks_create_pause_resume_run() {
    let mut c = loaded();
    send(
        &mut c,
        SsMsg::Tasks {
            session_id: 7,
            items: vec![task(1, "sched", "active"), task(2, "deleg", "done")],
        },
    );
    let v = c.view();
    assert_eq!(v.tasks.len(), 2);
    assert!(
        v.tasks[0].schedule.contains("0 9 * * *") && v.tasks[0].schedule.contains("Asia/Jerusalem")
    );
    assert_eq!(v.tasks[1].schedule, "");
    send(
        &mut c,
        SsMsg::Tasks {
            session_id: 99,
            items: vec![],
        },
    );
    assert_eq!(
        c.view().tasks.len(),
        2,
        "a list for a foreign session is not accepted"
    );

    let fx = send(
        &mut c,
        SsMsg::TaskCreate {
            kind: "sched".into(),
            title: "Report".into(),
            spec: "".into(),
            cron: Some("30 8 * * 1-5".into()),
        },
    );
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::CreateTask {
            session_id: 7,
            kind: "sched".into(),
            title: "Report".into(),
            spec: "Report".into(),
            cron: Some("30 8 * * 1-5".into())
        }],
        "an empty prompt falls back to the title"
    );
    for bad in [
        ("sched", "x", None),
        ("sched", "x", Some("* *")),
        ("deleg", "x", Some("0 9 * * *")),
        ("deleg", "  ", None),
        ("other", "x", None),
    ] {
        let fx = send(
            &mut c,
            SsMsg::TaskCreate {
                kind: bad.0.into(),
                title: bad.1.into(),
                spec: "s".into(),
                cron: bad.2.map(str::to_string),
            },
        );
        assert!(cmds(&fx).is_empty(), "{bad:?}");
        assert!(c.view().notice.is_some());
    }
    send(&mut c, SsMsg::TaskChanged(task(3, "deleg", "active")));
    assert_eq!(c.view().tasks.len(), 3);
    send(&mut c, SsMsg::TaskChanged(task(3, "deleg", "paused")));
    assert_eq!(c.view().tasks.len(), 3, "the same task is replaced");

    let fx = send(&mut c, SsMsg::TaskPause(1));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::SetTaskState {
            task_id: 1,
            state: "paused".into()
        }]
    );
    assert_eq!(c.view().tasks[0].state, "paused");
    let fx = send(&mut c, SsMsg::TaskResume(1));
    assert_eq!(
        cmds(&fx),
        vec![CoreCommand::SetTaskState {
            task_id: 1,
            state: "active".into()
        }]
    );
    assert!(
        cmds(&send(&mut c, SsMsg::TaskPause(404))).is_empty(),
        "there is no foreign task"
    );
    let fx = send(&mut c, SsMsg::TaskRun(1));
    assert_eq!(
        cmds(&fx),
        vec![
            CoreCommand::RunTask { task_id: 1 },
            CoreCommand::FetchTaskRuns { task_id: 1 }
        ]
    );
}

#[test]
fn task_runs_show_last_runs() {
    let mut c = loaded();
    send(
        &mut c,
        SsMsg::Tasks {
            session_id: 7,
            items: vec![task(1, "sched", "active")],
        },
    );
    let fx = send(&mut c, SsMsg::ShowRuns(1));
    assert_eq!(cmds(&fx), vec![CoreCommand::FetchTaskRuns { task_id: 1 }]);
    let runs = vec![
        RunItem {
            id: 5,
            task_id: 1,
            started_at: 100,
            status: "ok".into(),
            output: Some("<b>done</b>".into()),
        },
        RunItem {
            id: 4,
            task_id: 1,
            started_at: 90,
            status: "error".into(),
            output: None,
        },
    ];
    send(
        &mut c,
        SsMsg::Runs {
            task_id: 2,
            items: runs.clone(),
        },
    );
    assert!(
        c.view().runs.is_empty(),
        "runs of another task are not shown"
    );
    send(
        &mut c,
        SsMsg::Runs {
            task_id: 1,
            items: runs.clone(),
        },
    );
    let v = c.view();
    assert_eq!(v.runs_of, Some(1));
    assert_eq!(v.runs, runs);
}

#[test]
fn session_settings_follow_tree_updates_and_deletion() {
    use controllers::tree_types::TreeFrame;
    let mut c = loaded();
    let mut changed = session(7, Some(2), "New name");
    changed.cfg.mem = true;
    changed.cfg.notif = Some("important".into());
    changed.cfg.model = Some(4);
    send(&mut c, SsMsg::Tree(TreeFrame::SessionUpdated(changed)));
    let v = c.view();
    assert_eq!(
        (v.name.as_str(), v.mem, v.notif, v.model),
        ("New name", true, NotifMode::Important, Some(4))
    );
    send(&mut c, SsMsg::Tree(TreeFrame::SessionDeleted(7)));
    assert_eq!(
        c.view().session_id,
        None,
        "a deleted session closes the settings"
    );
}

#[test]
fn perm_table_set_session_replaces_only_the_same_row() {
    let mut t = PermTable::default();
    t.set_session(1, "send", Mode::Ask);
    t.set_session(2, "send", Mode::Deny);
    t.set_session(1, "shell", Mode::Deny);
    assert_eq!(
        t.rows.len(),
        3,
        "different sessions and actions are different rows"
    );
    t.set_session(1, "send", Mode::Deny);
    assert_eq!(t.rows.len(), 3, "the same row is replaced, not duplicated");
    assert_eq!(t.session(Some(1), "send"), Some(Mode::Deny));
    assert_eq!(t.session(Some(2), "send"), Some(Mode::Deny));
    assert_eq!(t.session(Some(1), "shell"), Some(Mode::Deny));
    t.set_session(2, "send", Mode::Ask);
    assert_eq!(
        t.session(Some(1), "send"),
        Some(Mode::Deny),
        "a foreign session is untouched"
    );
    assert_eq!(t.session(Some(2), "send"), Some(Mode::Ask));
}
