//! `TerminalController`: the user's terminal (VTE) in the session folder, one per session, and the agent output panel.
//! Agent commands are executed by the core in its own pty; here they are only displayed and never inserted into the user's VTE:
//! agent frames produce neither effects nor process launches.

use crate::time::Instant;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::controller::{Controller, Effect};
use crate::terminal_view::{
    AGENT_OUTPUT_LIMIT, AGENT_RUNS_KEPT, AgentRun, TermMsg, TerminalView, UserTermView, term_cwd,
};
use crate::tool_types::TermFrame;
use crate::tree_types::{SessionItem, TreeFrame};

#[derive(Debug, Default)]
struct Term {
    /// Folder of the running shell; `None` while there is no shell.
    spawned_in: Option<PathBuf>,
}

#[derive(Debug)]
pub struct TerminalController {
    home: PathBuf,
    session: Option<i64>,
    /// `id → (path, parent_id)`.
    sessions: HashMap<i64, (Option<String>, Option<i64>)>,
    terms: BTreeMap<i64, Term>,
    agent: HashMap<i64, Vec<AgentRun>>,
}

const FINAL: [&str; 4] = ["exited", "killed", "denied", "timeout"];

impl TerminalController {
    /// `home` is the user's home folder: the working folder for a session terminal without a path.
    pub fn new(home: PathBuf) -> Self {
        TerminalController {
            home,
            session: None,
            sessions: HashMap::new(),
            terms: BTreeMap::new(),
            agent: HashMap::new(),
        }
    }

    fn cwd_of(&self, session: i64) -> PathBuf {
        let path = self.sessions.get(&session).and_then(|s| s.0.as_deref());
        term_cwd(path, &self.home)
    }

    fn user_view(&self, id: i64) -> Option<UserTermView> {
        let term = self.terms.get(&id)?;
        Some(UserTermView {
            session_id: id,
            cwd: term.spawned_in.clone().unwrap_or_else(|| self.cwd_of(id)),
            running: term.spawned_in.is_some(),
            needs_spawn: term.spawned_in.is_none(),
        })
    }

    fn forget(&mut self, id: i64) {
        let gone: Vec<i64> = self
            .sessions
            .iter()
            .filter(|(sid, (_, parent))| **sid == id || *parent == Some(id))
            .map(|(sid, _)| *sid)
            .chain([id])
            .collect();
        for g in gone {
            self.sessions.remove(&g);
            self.terms.remove(&g);
            self.agent.remove(&g);
        }
    }

    fn set_session(&mut self, s: &SessionItem) {
        self.sessions.insert(s.id, (s.path.clone(), s.parent_id));
    }

    fn run_mut(&mut self, session: i64, id: &str) -> &mut AgentRun {
        let runs = self.agent.entry(session).or_default();
        let at = match runs.iter().position(|r| r.request_id == id) {
            Some(i) => i,
            None => {
                runs.push(AgentRun {
                    request_id: id.to_string(),
                    command: String::new(),
                    cwd: String::new(),
                    status: "queued".to_string(),
                    exit_code: None,
                    duration_ms: None,
                    output: String::new(),
                    truncated: false,
                    next_seq: 0,
                });
                if runs.len() > AGENT_RUNS_KEPT {
                    runs.remove(0);
                }
                runs.len() - 1
            }
        };
        &mut runs[at]
    }

    fn agent_frame(&mut self, session: i64, frame: TermFrame) {
        match frame {
            TermFrame::Command {
                request_id,
                command,
                cwd,
            } => {
                let existed = self
                    .agent
                    .get(&session)
                    .is_some_and(|r| r.iter().any(|x| x.request_id == request_id));
                let run = self.run_mut(session, &request_id);
                if !existed || run.command.is_empty() {
                    run.command = command;
                    run.cwd = cwd;
                }
            }
            TermFrame::Output {
                request_id,
                seq,
                data,
                ..
            } => {
                let run = self.run_mut(session, &request_id);
                if seq < run.next_seq {
                    return;
                }
                run.next_seq = seq + 1;
                if run.truncated {
                    return;
                }
                let room = AGENT_OUTPUT_LIMIT - run.output.len();
                let mut end = data.len().min(room);
                while !data.is_char_boundary(end) {
                    end -= 1;
                }
                run.output.push_str(&data[..end]);
                run.truncated = end < data.len();
            }
            TermFrame::Status {
                request_id,
                status,
                exit_code,
                duration_ms,
            } => {
                let run = self.run_mut(session, &request_id);
                if FINAL.contains(&run.status.as_str()) && !FINAL.contains(&status.as_str()) {
                    return;
                }
                run.status = status;
                run.exit_code = exit_code.or(run.exit_code);
                run.duration_ms = duration_ms.or(run.duration_ms);
            }
        }
    }
}

impl Controller for TerminalController {
    type Msg = TermMsg;
    type View = TerminalView;

    fn update(&mut self, msg: TermMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            TermMsg::Session(None) => self.session = None,
            TermMsg::Session(Some(id)) => {
                self.session = Some(id);
                self.terms.entry(id).or_default();
            }
            TermMsg::Snapshot(items) => {
                if !items.is_empty() {
                    self.sessions.clear();
                    for s in &items {
                        self.set_session(s);
                    }
                    let known = &self.sessions;
                    self.terms.retain(|id, _| known.contains_key(id));
                }
            }
            TermMsg::Tree(TreeFrame::SessionCreated(s) | TreeFrame::SessionUpdated(s)) => {
                self.set_session(&s);
            }
            TermMsg::Tree(TreeFrame::SessionDeleted(id)) => self.forget(id),
            TermMsg::Tree(_) => {}
            TermMsg::Spawned(id) => {
                let cwd = self.cwd_of(id);
                self.terms.entry(id).or_default().spawned_in = Some(cwd);
            }
            TermMsg::Exited(id) => {
                if let Some(t) = self.terms.get_mut(&id) {
                    t.spawned_in = None;
                }
            }
            TermMsg::Agent { session_id, frame } => self.agent_frame(session_id, frame),
        }
        Vec::new()
    }

    fn view(&self) -> TerminalView {
        TerminalView {
            session: self.session,
            user: self.session.and_then(|s| self.user_view(s)),
            live: self
                .terms
                .iter()
                .filter(|(_, t)| t.spawned_in.is_some())
                .map(|(id, _)| *id)
                .collect(),
            agent: self
                .session
                .and_then(|s| self.agent.get(&s).cloned())
                .unwrap_or_default(),
        }
    }
}
