//! "Delegation and tasks" block: the session's task list, creating "scheduled" (cron, Asia/Jerusalem) and "delegate" tasks,
//! pause and resume, run, recent runs. Titles and output are shown without markup.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use controllers::i18n::{tr, tr_args};
use controllers::ss_view::{SsMsg, SsView, TaskRow};
use controllers::tool_types::RunItem;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

pub struct TasksBlock {
    pub root: gtk::Box,
    pub kind: gtk::DropDown,
    pub title: gtk::Entry,
    pub cron: gtk::Entry,
    pub create: gtk::Button,
    list: gtk::Box,
    runs: gtk::Label,
    shown: RefCell<(Vec<TaskRow>, Option<i64>, Vec<RunItem>)>,
    send: Rc<dyn Fn(SsMsg)>,
    flag: Cell<bool>,
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_use_markup(false);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

fn button(text: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("shell-btn");
    a11y::set_name(&b, text);
    b
}

/// A run row: time, status, output.
pub fn run_text(r: &RunItem) -> String {
    tr_args(
        "ss-run-line",
        &[
            ("id", &r.id.to_string()),
            ("status", &r.status),
            ("output", r.output.as_deref().unwrap_or("")),
        ],
    )
}

impl TasksBlock {
    pub fn new(send: Rc<dyn Fn(SsMsg)>) -> Rc<TasksBlock> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let heading = label(&tr("ss-section-tasks"));
        heading.add_css_class("heading");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        a11y::set_name(&list, &tr("ss-section-tasks"));
        let kind = gtk::DropDown::from_strings(&[&tr("ss-task-sched"), &tr("ss-task-deleg")]);
        a11y::set_name(&kind, &tr("ss-task-kind"));
        let title = gtk::Entry::new();
        title.set_placeholder_text(Some(&tr("ss-task-title")));
        a11y::set_name(&title, &tr("ss-task-title"));
        let cron = gtk::Entry::new();
        cron.set_placeholder_text(Some(&tr("ss-task-cron")));
        a11y::set_name(&cron, &tr("ss-task-cron"));
        let create = button(&tr("ss-task-create"));
        let tz = label(&tr("ss-task-tz"));
        tz.add_css_class("dim-label");
        let form = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [
            kind.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            cron.upcast_ref(),
            create.upcast_ref(),
        ] {
            form.append(w);
        }
        let runs = label("");
        a11y::set_name(&runs, &tr("ss-runs-name"));
        for w in [
            heading.upcast_ref::<gtk::Widget>(),
            list.upcast_ref(),
            form.upcast_ref(),
            tz.upcast_ref(),
            runs.upcast_ref(),
        ] {
            root.append(w);
        }
        let block = Rc::new(TasksBlock {
            root,
            kind,
            title,
            cron,
            create,
            list,
            runs,
            shown: RefCell::new((Vec::new(), None, Vec::new())),
            send,
            flag: Cell::new(false),
        });
        let weak = Rc::downgrade(&block);
        block.create.connect_clicked(move |_| {
            let Some(b) = weak.upgrade() else { return };
            let sched = b.kind.selected() == 0;
            let cron = b.cron.text().trim().to_string();
            (b.send)(SsMsg::TaskCreate {
                kind: if sched { "sched" } else { "deleg" }.to_string(),
                title: b.title.text().to_string(),
                spec: String::new(),
                cron: sched.then_some(cron),
            });
            b.title.set_text("");
        });
        block
    }

    pub fn apply(&self, v: &SsView) {
        self.flag.set(true);
        let key = (v.tasks.clone(), v.runs_of, v.runs.clone());
        if *self.shown.borrow() != key {
            *self.shown.borrow_mut() = key;
            self.rebuild(v);
        }
        self.flag.set(false);
    }

    fn rebuild(&self, v: &SsView) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        for t in &v.tasks {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let state = match t.state.as_str() {
                "paused" => tr("ss-task-state-paused"),
                "done" => tr("ss-task-state-done"),
                _ => tr("ss-task-state-active"),
            };
            let text = label(&format!("{} · {} {}", t.title, state, t.schedule));
            text.set_hexpand(true);
            row.append(&text);
            let pause = button(&if t.state == "paused" {
                tr("ss-task-resume")
            } else {
                tr("ss-task-pause")
            });
            let (s, id, paused) = (self.send.clone(), t.id, t.state == "paused");
            pause.connect_clicked(move |_| {
                s(if paused {
                    SsMsg::TaskResume(id)
                } else {
                    SsMsg::TaskPause(id)
                })
            });
            let run = button(&tr("ss-task-run"));
            let s = self.send.clone();
            run.connect_clicked(move |_| s(SsMsg::TaskRun(id)));
            let runs = button(&tr("ss-task-runs"));
            let s = self.send.clone();
            runs.connect_clicked(move |_| s(SsMsg::ShowRuns(id)));
            row.append(&pause);
            row.append(&run);
            row.append(&runs);
            self.list.append(&row);
        }
        let text: Vec<String> = v.runs.iter().map(run_text).collect();
        self.runs.set_text(&text.join("\n"));
        self.runs.set_visible(v.runs_of.is_some());
    }

    /// Task titles on screen (for UI scenarios).
    pub fn task_rows(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut child = self.list.first_child();
        while let Some(c) = child {
            if let Some(l) = c.first_child().and_downcast::<gtk::Label>() {
                out.push(l.text().to_string());
            }
            child = c.next_sibling();
        }
        out
    }

    pub fn runs_text(&self) -> String {
        self.runs.text().to_string()
    }
}
