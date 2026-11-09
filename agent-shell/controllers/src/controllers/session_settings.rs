//! `SessionSettingsController`: session settings, permissions (tightening only), tasks and dangerous actions.
//! Session permissions never loosen global and persona ones: a loosening value is unavailable in the view and is not sent to the core.

use crate::time::Instant;
use std::collections::HashMap;

use crate::controller::{CfgPatch, Controller, CoreCommand, Effect};
use crate::i18n::{tr, tr_args};
use crate::permissions::{Mode, PermTable, effective_for_action, effective_mode};
use crate::ss_view::{
    ConfirmKind, ConfirmView, ModelChoice, NotifMode, PERM_ACTIONS, PermField, PermOption, SsMsg,
    SsView, TaskRow,
};
use crate::tool_types::{RunItem, TaskItem};
use crate::tree_types::{PersonaItem, SessionItem, TreeFrame};

const TITLE_MAX: usize = 200;

#[derive(Debug, Default)]
pub struct SessionSettingsController {
    session: Option<i64>,
    sessions: HashMap<i64, SessionItem>,
    personas: Vec<PersonaItem>,
    models: Vec<ModelChoice>,
    perms: PermTable,
    tasks: Vec<TaskItem>,
    runs_of: Option<i64>,
    runs: Vec<RunItem>,
    confirm: Option<ConfirmKind>,
    notice: Option<String>,
    exported: Option<String>,
}

fn cmd(c: CoreCommand) -> Effect {
    Effect::Command(c)
}

fn cron_ok(c: &str) -> bool {
    c.split_whitespace().count() == 5
}

impl SessionSettingsController {
    pub fn new() -> Self {
        Self::default()
    }

    fn cur(&self) -> Option<&SessionItem> {
        self.sessions.get(&self.session?)
    }

    fn cur_mut(&mut self) -> Option<&mut SessionItem> {
        let id = self.session?;
        self.sessions.get_mut(&id)
    }

    /// Rule of the upper levels (global and persona) for an action: `(no rules → Allow, the default)`.
    fn upper(&self, action: &str) -> (Mode, Mode) {
        let persona = self.cur().and_then(|s| s.persona_id);
        let (g, p) = (
            self.perms.global(action),
            self.perms.persona(persona, action),
        );
        (
            effective_mode(g, p, None),
            effective_for_action(action, g, p, None),
        )
    }

    fn perm_field(&self, id: i64, action: &str) -> PermField {
        let persona = self.cur().and_then(|s| s.persona_id);
        let (floor, as_persona) = self.upper(action);
        let row = self.perms.session(Some(id), action);
        let options: Vec<PermOption> = [Mode::Allow, Mode::Ask, Mode::Deny]
            .into_iter()
            .map(|mode| PermOption {
                mode,
                label: match mode {
                    Mode::Allow => tr("chat-mode-allow"),
                    Mode::Ask => tr("chat-mode-ask"),
                    Mode::Deny => tr("chat-mode-deny"),
                },
                enabled: mode >= floor,
            })
            .collect();
        let locked = floor == Mode::Deny;
        let any_off = options.iter().any(|o| !o.enabled);
        PermField {
            action: action.to_string(),
            label: match action {
                "send" => tr("ss-perm-send"),
                "delete" => tr("ss-perm-delete"),
                _ => tr("ss-perm-shell"),
            },
            selected: row.filter(|m| *m != as_persona),
            effective: self.perms.effective(action, persona, Some(id)),
            options,
            locked,
            note: if locked {
                Some(tr("ss-perm-locked"))
            } else {
                any_off.then(|| tr("ss-perm-tighten-only"))
            },
        }
    }

    fn set_perm(&mut self, action: &str, mode: Option<Mode>, fx: &mut Vec<Effect>) {
        let Some(id) = self.session else { return };
        if !PERM_ACTIONS.contains(&action) {
            return;
        }
        let (floor, as_persona) = self.upper(action);
        let mode = mode.unwrap_or(as_persona);
        if mode < floor {
            self.notice = Some(tr("ss-perm-loosen-blocked"));
            return;
        }
        self.notice = None;
        self.perms.set_session(id, action, mode);
        fx.push(cmd(CoreCommand::PutSessionPerm {
            session_id: id,
            action: action.to_string(),
            mode,
        }));
    }

    fn patch_cfg(&mut self, patch: CfgPatch, fx: &mut Vec<Effect>) {
        let Some(id) = self.session else { return };
        if let Some(s) = self.cur_mut() {
            if let Some(c) = &patch.ctx {
                s.cfg.ctx = Some(c.clone());
            }
            if let Some(m) = patch.model {
                s.cfg.model = m;
            }
            if let Some(m) = patch.mem {
                s.cfg.mem = m;
            }
            if let Some(a) = patch.auto {
                s.cfg.auto = a;
            }
            if let Some(n) = &patch.notif {
                s.cfg.notif = Some(n.clone());
            }
        }
        fx.push(cmd(CoreCommand::PatchSessionCfg {
            session_id: id,
            cfg: patch,
        }));
    }

    fn ask(&mut self, kind: ConfirmKind) {
        if self.session.is_some() {
            self.confirm = Some(kind);
        }
    }

    fn confirm_view(&self, kind: ConfirmKind) -> Option<ConfirmView> {
        let s = self.cur()?;
        let name = s.name.as_str();
        Some(match kind {
            ConfirmKind::Clear => ConfirmView {
                kind,
                title: tr("ss-confirm-clear-title"),
                lines: vec![
                    tr_args("ss-confirm-clear-line1", &[("name", name)]),
                    tr("ss-confirm-clear-line2"),
                ],
            },
            ConfirmKind::Delete => {
                let threads = self
                    .sessions
                    .values()
                    .filter(|t| t.parent_id == Some(s.id))
                    .count()
                    .to_string();
                ConfirmView {
                    kind,
                    title: tr("ss-confirm-delete-title"),
                    lines: vec![
                        tr_args("ss-confirm-delete-line1", &[("name", name)]),
                        tr_args("ss-confirm-delete-line2", &[("count", &threads)]),
                        tr("ss-confirm-delete-line3"),
                        tr("ss-confirm-delete-line4"),
                    ],
                }
            }
        })
    }

    fn task_create(
        &mut self,
        (kind, title, spec, cron): (String, String, String, Option<String>),
        fx: &mut Vec<Effect>,
    ) {
        let Some(id) = self.session else { return };
        let title = title.trim().to_string();
        let ok = !title.is_empty()
            && title.chars().count() <= TITLE_MAX
            && match (kind.as_str(), cron.as_deref()) {
                ("sched", Some(c)) => cron_ok(c),
                ("deleg", None) => true,
                _ => false,
            };
        if !ok {
            self.notice = Some(tr("ss-task-invalid"));
            return;
        }
        self.notice = None;
        let spec = if spec.trim().is_empty() {
            title.clone()
        } else {
            spec
        };
        fx.push(cmd(CoreCommand::CreateTask {
            session_id: id,
            kind,
            title,
            spec,
            cron,
        }));
    }

    fn set_task_state(&mut self, task_id: i64, state: &str, fx: &mut Vec<Effect>) {
        let Some(t) = self.tasks.iter_mut().find(|t| t.id == task_id) else {
            return;
        };
        t.state = state.to_string();
        fx.push(cmd(CoreCommand::SetTaskState {
            task_id,
            state: state.to_string(),
        }));
    }

    fn tree(&mut self, frame: TreeFrame) {
        match frame {
            TreeFrame::SessionCreated(s) | TreeFrame::SessionUpdated(s) => {
                self.sessions.insert(s.id, s);
            }
            TreeFrame::SessionDeleted(id) => {
                let gone: Vec<i64> = self
                    .sessions
                    .values()
                    .filter(|s| s.id == id || s.parent_id == Some(id))
                    .map(|s| s.id)
                    .collect();
                for g in gone {
                    self.sessions.remove(&g);
                    if self.session == Some(g) {
                        self.close();
                    }
                }
            }
            _ => {}
        }
    }

    fn close(&mut self) {
        self.session = None;
        self.tasks.clear();
        self.runs_of = None;
        self.runs.clear();
        self.confirm = None;
        self.notice = None;
        self.exported = None;
    }
}

impl Controller for SessionSettingsController {
    type Msg = SsMsg;
    type View = SsView;

    fn update(&mut self, msg: SsMsg, _now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match msg {
            SsMsg::Open(None) => self.close(),
            SsMsg::Open(Some(id)) => {
                self.close();
                self.session = Some(id);
                fx.push(cmd(CoreCommand::FetchTasks { session_id: id }));
                fx.push(cmd(CoreCommand::FetchPermissions));
            }
            SsMsg::Snapshot(items) => {
                self.sessions = items.into_iter().map(|s| (s.id, s)).collect();
            }
            SsMsg::Tree(frame) => self.tree(frame),
            SsMsg::Personas(p) => self.personas = p,
            SsMsg::Models(m) => self.models = m,
            SsMsg::Perms(p) => self.perms = p,
            SsMsg::PermRejected => {
                self.notice = Some(tr("ss-perm-rejected"));
                fx.push(cmd(CoreCommand::FetchPermissions));
            }
            SsMsg::Tasks { session_id, items } => {
                if self.session == Some(session_id) {
                    self.tasks = items;
                }
            }
            SsMsg::Runs { task_id, items } => {
                if self.runs_of == Some(task_id) {
                    self.runs = items;
                }
            }
            SsMsg::TaskChanged(t) => match self.tasks.iter_mut().find(|x| x.id == t.id) {
                Some(x) => *x = t,
                None => self.tasks.push(t),
            },
            SsMsg::SetPersona(p) => {
                if let (Some(id), Some(s)) = (self.session, self.cur_mut()) {
                    s.persona_id = p;
                    fx.push(cmd(CoreCommand::SetSessionPersona {
                        session_id: id,
                        persona_id: p,
                    }));
                }
            }
            SsMsg::SetPath(path) => {
                if let (Some(id), Some(s)) = (self.session, self.cur_mut()) {
                    s.path = Some(path.clone());
                    fx.push(cmd(CoreCommand::SetSessionPath {
                        session_id: id,
                        path,
                    }));
                }
            }
            SsMsg::MoveTo(folder_id) => {
                if let (Some(id), Some(s)) = (self.session, self.cur_mut()) {
                    s.folder_id = folder_id;
                    fx.push(cmd(CoreCommand::MoveSession { id, folder_id }));
                }
            }
            SsMsg::SetCtxInherit(inherit) => {
                if self.cur().is_some_and(|s| s.parent_id.is_some()) {
                    let ctx = if inherit { "parent" } else { "none" };
                    self.patch_cfg(
                        CfgPatch {
                            ctx: Some(ctx.into()),
                            ..CfgPatch::default()
                        },
                        &mut fx,
                    );
                }
            }
            SsMsg::SetModel(m) => self.patch_cfg(
                CfgPatch {
                    model: Some(m),
                    ..CfgPatch::default()
                },
                &mut fx,
            ),
            SsMsg::SetMem(m) => self.patch_cfg(
                CfgPatch {
                    mem: Some(m),
                    ..CfgPatch::default()
                },
                &mut fx,
            ),
            SsMsg::SetAuto(a) => self.patch_cfg(
                CfgPatch {
                    auto: Some(a),
                    ..CfgPatch::default()
                },
                &mut fx,
            ),
            SsMsg::SetNotif(n) => self.patch_cfg(
                CfgPatch {
                    notif: Some(n.as_str().into()),
                    ..CfgPatch::default()
                },
                &mut fx,
            ),
            SsMsg::SetPerm { action, mode } => self.set_perm(&action, mode, &mut fx),
            SsMsg::NewThread => {
                if let Some(s) = self.cur().filter(|s| s.parent_id.is_none()) {
                    fx.push(cmd(CoreCommand::CreateThread {
                        parent_id: s.id,
                        name: tr("ss-new-thread-name"),
                    }));
                }
            }
            SsMsg::AskClear => self.ask(ConfirmKind::Clear),
            SsMsg::AskDelete => self.ask(ConfirmKind::Delete),
            SsMsg::Cancel => self.confirm = None,
            SsMsg::Confirm => {
                if let (Some(kind), Some(id)) = (self.confirm.take(), self.session) {
                    fx.push(cmd(match kind {
                        ConfirmKind::Clear => CoreCommand::ClearSession { session_id: id },
                        ConfirmKind::Delete => CoreCommand::DeleteSession { id },
                    }));
                }
            }
            SsMsg::Export => {
                if let Some(id) = self.session {
                    fx.push(cmd(CoreCommand::ExportSession { session_id: id }));
                }
            }
            SsMsg::Exported { path, .. } => self.exported = Some(path),
            SsMsg::TaskCreate {
                kind,
                title,
                spec,
                cron,
            } => self.task_create((kind, title, spec, cron), &mut fx),
            SsMsg::TaskPause(id) => self.set_task_state(id, "paused", &mut fx),
            SsMsg::TaskResume(id) => self.set_task_state(id, "active", &mut fx),
            SsMsg::TaskRun(id) => {
                if self.tasks.iter().any(|t| t.id == id) {
                    self.runs_of = Some(id);
                    fx.push(cmd(CoreCommand::RunTask { task_id: id }));
                    fx.push(cmd(CoreCommand::FetchTaskRuns { task_id: id }));
                }
            }
            SsMsg::ShowRuns(id) => {
                if self.tasks.iter().any(|t| t.id == id) {
                    self.runs_of = Some(id);
                    self.runs.clear();
                    fx.push(cmd(CoreCommand::FetchTaskRuns { task_id: id }));
                }
            }
        }
        fx
    }

    fn view(&self) -> SsView {
        let s = self.cur();
        let cfg = s.map(|s| &s.cfg);
        SsView {
            session_id: self.session,
            name: s.map_or_else(String::new, |s| s.name.clone()),
            is_thread: s.is_some_and(|s| s.parent_id.is_some()),
            persona: s.and_then(|s| s.persona_id),
            personas: self
                .personas
                .iter()
                .map(|p| (p.id, p.name.clone()))
                .collect(),
            path: s.and_then(|s| s.path.clone()).unwrap_or_default(),
            folder_id: s.and_then(|s| s.folder_id),
            inherit_ctx: cfg.is_none_or(|c| c.ctx.as_deref() != Some("none")),
            model: cfg.and_then(|c| c.model),
            models: self.models.clone(),
            mem: cfg.is_some_and(|c| c.mem),
            auto: cfg.is_some_and(|c| c.auto),
            notif: NotifMode::parse(cfg.and_then(|c| c.notif.as_deref())),
            perms: self
                .session
                .map(|id| {
                    PERM_ACTIONS
                        .iter()
                        .map(|a| self.perm_field(id, a))
                        .collect()
                })
                .unwrap_or_default(),
            tasks: self
                .tasks
                .iter()
                .map(|t| TaskRow {
                    id: t.id,
                    kind: t.kind.clone(),
                    title: t.title.clone(),
                    state: t.state.clone(),
                    schedule: t
                        .cron
                        .as_ref()
                        .map_or_else(String::new, |c| format!("{c} · {}", t.tz)),
                })
                .collect(),
            runs_of: self.runs_of,
            runs: self.runs.clone(),
            confirm: self.confirm.and_then(|k| self.confirm_view(k)),
            notice: self.notice.clone(),
            exported: self.exported.clone(),
        }
    }
}
