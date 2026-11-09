//! "Terminal" tab: the user's VTE in the session folder (one per session, the process lives on
//! switching) and a read-only "Agent output" panel with core frames. Agent commands are never inserted here:
//! the core runs them in its own pty. The terminal environment is only `user_terminal_env` (no tokens).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use controllers::i18n::{tr, tr_args};
use controllers::terminal_view::{AgentRun, TermMsg, TerminalView, user_shell, user_terminal_env};
use gtk4 as gtk;
use gtk4::prelude::*;
use vte4::prelude::*;

use crate::a11y;

pub struct TerminalPane {
    pub root: gtk::Box,
    pub cwd: gtk::Label,
    pub agent_text: gtk::TextView,
    stack: gtk::Stack,
    terms: RefCell<HashMap<i64, vte4::Terminal>>,
    /// Shell start has begun and is not confirmed yet: do not start again.
    spawning: RefCell<HashSet<i64>>,
    shown_agent: RefCell<Vec<AgentRun>>,
    send: Rc<dyn Fn(TermMsg)>,
}

/// Agent panel text: command, status, output; no markup.
pub fn agent_text(runs: &[AgentRun]) -> String {
    if runs.is_empty() {
        return tr("terminal-agent-empty");
    }
    let mut out = String::new();
    for r in runs {
        out.push_str(&tr_args(
            "terminal-agent-run",
            &[("command", &r.command), ("status", &r.status)],
        ));
        out.push('\n');
        out.push_str(&r.output);
        if !r.output.ends_with('\n') && !r.output.is_empty() {
            out.push('\n');
        }
    }
    out
}

impl TerminalPane {
    pub fn new(send: Rc<dyn Fn(TermMsg)>) -> Rc<TerminalPane> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let cwd = gtk::Label::new(None);
        cwd.set_use_markup(false);
        cwd.set_xalign(0.0);
        cwd.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        cwd.set_margin_start(8);
        cwd.set_margin_end(8);
        cwd.add_css_class("terminal-cwd");
        a11y::set_name(&cwd, &tr("terminal-cwd-name"));
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let agent_title = gtk::Label::new(Some(&tr("terminal-agent-title")));
        agent_title.set_use_markup(false);
        agent_title.set_xalign(0.0);
        agent_title.set_margin_start(8);
        agent_title.add_css_class("terminal-agent-title");
        let agent_text = gtk::TextView::new();
        agent_text.set_editable(false);
        agent_text.set_cursor_visible(false);
        agent_text.set_monospace(true);
        agent_text.set_wrap_mode(gtk::WrapMode::WordChar);
        a11y::set_name(&agent_text, &tr("terminal-agent-title"));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&agent_text));
        scroll.set_min_content_height(120);
        for w in [
            cwd.upcast_ref::<gtk::Widget>(),
            stack.upcast_ref(),
            agent_title.upcast_ref(),
            scroll.upcast_ref(),
        ] {
            root.append(w);
        }
        Rc::new(TerminalPane {
            root,
            cwd,
            agent_text,
            stack,
            terms: RefCell::new(HashMap::new()),
            spawning: RefCell::new(HashSet::new()),
            shown_agent: RefCell::new(Vec::new()),
            send,
        })
    }

    fn terminal(self: &Rc<Self>, session_id: i64) -> vte4::Terminal {
        if let Some(t) = self.terms.borrow().get(&session_id) {
            return t.clone();
        }
        let term = vte4::Terminal::new();
        term.set_vexpand(true);
        term.set_scrollback_lines(10_000);
        a11y::set_name(&term, &tr("tools-tab-terminal"));
        let send = self.send.clone();
        term.connect_child_exited(move |_, _| send(TermMsg::Exited(session_id)));
        self.stack.add_named(&term, Some(&format!("t{session_id}")));
        self.terms.borrow_mut().insert(session_id, term.clone());
        term
    }

    fn spawn(self: &Rc<Self>, session_id: i64, cwd: &Path) {
        if !self.spawning.borrow_mut().insert(session_id) {
            return;
        }
        let term = self.terminal(session_id);
        let parent: Vec<(String, String)> = std::env::vars().collect();
        // VTE only adds `envv` to the GUI environment, so the shell is started via `env -i`: its environment is exactly
        // `user_terminal_env` (no tokens, display or bus)
        let mut argv: Vec<String> = vec!["/usr/bin/env".into(), "-i".into()];
        argv.extend(
            user_terminal_env(&parent, cwd)
                .into_iter()
                .map(|(k, v)| format!("{k}={v}")),
        );
        argv.push(user_shell(std::env::var("SHELL").ok().as_deref()));
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let work = if cwd.is_dir() {
            cwd.to_string_lossy().into_owned()
        } else {
            std::env::var("HOME").unwrap_or_else(|_| "/".into())
        };
        let (send, me) = (self.send.clone(), Rc::downgrade(self));
        term.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(&work),
            &argv_refs,
            &[],
            gtk::glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gtk::gio::Cancellable>,
            move |res| {
                if let Some(pane) = me.upgrade() {
                    pane.spawning.borrow_mut().remove(&session_id);
                }
                match res {
                    Ok(_) => send(TermMsg::Spawned(session_id)),
                    Err(e) => gtk::glib::g_warning!("agent-shell", "terminal not started: {e}"),
                }
            },
        );
    }

    /// Applies the view: the active session's terminal (started if needed, only while the tab is shown), the agent panel.
    pub fn apply(self: &Rc<Self>, view: &TerminalView, shown: bool) {
        if let Some(user) = &view.user {
            self.cwd.set_text(&user.cwd.to_string_lossy());
            let known = self.terms.borrow().contains_key(&user.session_id);
            if user.needs_spawn && shown {
                self.spawn(user.session_id, &user.cwd);
            } else if known {
                self.stack
                    .set_visible_child_name(&format!("t{}", user.session_id));
            }
            if shown && (user.needs_spawn || !known) {
                let _ = self.terminal(user.session_id);
                self.stack
                    .set_visible_child_name(&format!("t{}", user.session_id));
            }
        } else {
            self.cwd.set_text("");
        }
        if *self.shown_agent.borrow() != view.agent {
            *self.shown_agent.borrow_mut() = view.agent.clone();
            self.agent_text.buffer().set_text(&agent_text(&view.agent));
            let buffer = self.agent_text.buffer();
            let end = buffer.end_iter();
            let mark = buffer.create_mark(None, &end, false);
            self.agent_text.scroll_mark_onscreen(&mark);
        }
    }

    /// The session terminal was created (for UI scenarios).
    pub fn has_terminal(&self, session_id: i64) -> bool {
        self.terms.borrow().contains_key(&session_id)
    }

    /// Type text into the session terminal as from the keyboard (for UI scenarios).
    pub fn type_text(&self, session_id: i64, text: &str) {
        if let Some(t) = self.terms.borrow().get(&session_id) {
            t.feed_child(text.as_bytes());
        }
    }

    pub fn agent_buffer_text(&self) -> String {
        let b = self.agent_text.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }
}
