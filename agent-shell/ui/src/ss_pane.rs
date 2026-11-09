//! "Session settings" tab: persona, folder, context, model, memory, autonomous mode, notifications,
//! permissions, tasks and dangerous actions with confirmation. `SessionSettingsController` decides the values; widgets are persistent,
//! the view switches them without echo (`flag`) so focus and keyboard are not lost.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use controllers::i18n::tr;
use controllers::ss_view::{NotifMode, SsMsg, SsView};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::ss_perms::PermsBlock;
use crate::ss_tasks::TasksBlock;

pub struct SettingsPane {
    pub root: gtk::ScrolledWindow,
    pub name: gtk::Label,
    pub persona: gtk::DropDown,
    pub path: gtk::Entry,
    pub ctx: gtk::Switch,
    pub model: gtk::DropDown,
    pub mem: gtk::Switch,
    pub auto: gtk::Switch,
    pub notif: gtk::DropDown,
    pub perms: Rc<PermsBlock>,
    pub tasks: Rc<TasksBlock>,
    pub clear: gtk::Button,
    pub delete: gtk::Button,
    pub export: gtk::Button,
    pub new_thread: gtk::Button,
    pub move_to: gtk::Button,
    pub(crate) ctx_row: gtk::Box,
    pub(crate) notice: gtk::Label,
    pub(crate) exported: gtk::Label,
    pub(crate) confirm: gtk::Box,
    pub(crate) confirm_title: gtk::Label,
    pub(crate) confirm_lines: gtk::Box,
    pub(crate) personas: RefCell<Vec<(i64, String)>>,
    pub(crate) models: RefCell<Vec<(i64, String)>>,
    pub(crate) flag: Rc<Cell<bool>>,
    pub(crate) session: Cell<Option<i64>>,
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_use_markup(false);
    l.set_xalign(0.0);
    l
}

fn heading(text: &str) -> gtk::Label {
    let l = label(text);
    l.add_css_class("heading");
    l
}

fn row(title: &str, control: &(impl IsA<gtk::Widget> + IsA<gtk::Accessible>)) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(title);
    l.set_hexpand(true);
    b.append(&l);
    b.append(control);
    a11y::set_name(control, title);
    b
}

fn button(text: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("shell-btn");
    a11y::set_name(&b, text);
    b
}

fn drop_down(items: &[String]) -> gtk::DropDown {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    gtk::DropDown::from_strings(&refs)
}

impl SettingsPane {
    pub fn new(send: Rc<dyn Fn(SsMsg)>, on_move: Rc<dyn Fn(i64)>) -> Rc<SettingsPane> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(8);
        content.set_margin_bottom(12);
        let name = heading("");
        let persona = drop_down(&[tr("ss-persona-default")]);
        let path = gtk::Entry::new();
        path.set_hexpand(true);
        a11y::set_name(&path, &tr("ss-path"));
        let pick = button(&tr("ss-path-pick"));
        let move_to = button(&tr("ss-move"));
        let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        path_row.append(&path);
        path_row.append(&pick);
        let ctx = gtk::Switch::new();
        let ctx_row = row(&tr("ss-ctx"), &ctx);
        let model = drop_down(&[tr("ss-model-default")]);
        let mem = gtk::Switch::new();
        let auto = gtk::Switch::new();
        let notif = drop_down(&[
            tr("ss-notif-all"),
            tr("ss-notif-important"),
            tr("ss-notif-off"),
        ]);
        let perms = PermsBlock::new(send.clone());
        let tasks = TasksBlock::new(send.clone());
        let new_thread = button(&tr("ss-new-thread"));
        let clear = button(&tr("ss-clear"));
        let export = button(&tr("ss-export"));
        let delete = button(&tr("ss-delete"));
        delete.add_css_class("destructive-action");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for b in [&new_thread, &clear, &export, &delete] {
            actions.append(b);
        }
        let notice = label("");
        notice.set_wrap(true);
        let exported = label("");
        let confirm = gtk::Box::new(gtk::Orientation::Vertical, 6);
        confirm.add_css_class("card");
        confirm.set_visible(false);
        a11y::set_name(&confirm, &tr("ss-confirm-name"));
        let confirm_title = heading("");
        let confirm_lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let yes = button(&tr("ss-confirm-yes"));
        let no = button(&tr("ss-confirm-no"));
        let yn = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        yn.append(&yes);
        yn.append(&no);
        confirm.append(&confirm_title);
        confirm.append(&confirm_lines);
        confirm.append(&yn);
        content.append(&name);
        content.append(&row(&tr("ss-persona"), &persona));
        content.append(&label(&tr("ss-path")));
        content.append(&path_row);
        content.append(&move_to);
        content.append(&ctx_row);
        content.append(&row(&tr("ss-model"), &model));
        content.append(&row(&tr("ss-mem"), &mem));
        content.append(&row(&tr("ss-auto"), &auto));
        content.append(&row(&tr("ss-notif"), &notif));
        content.append(&perms.root);
        content.append(&tasks.root);
        content.append(&heading(&tr("ss-section-actions")));
        content.append(&actions);
        content.append(&confirm);
        content.append(&exported);
        content.append(&notice);
        let root = gtk::ScrolledWindow::new();
        root.set_vexpand(true);
        root.set_child(Some(&content));
        let pane = Rc::new(SettingsPane {
            root,
            name,
            persona,
            path,
            ctx,
            model,
            mem,
            auto,
            notif,
            perms,
            tasks,
            clear,
            delete,
            export,
            new_thread,
            move_to,
            ctx_row,
            notice,
            exported,
            confirm,
            confirm_title,
            confirm_lines,
            personas: RefCell::new(Vec::new()),
            models: RefCell::new(Vec::new()),
            flag: Rc::new(Cell::new(false)),
            session: Cell::new(None),
        });
        pane.wire(&send, &on_move, &pick, &yes, &no);
        pane
    }

    fn sync_drop(
        &self,
        dd: &gtk::DropDown,
        first: &str,
        items: &[(i64, String)],
        current: &RefCell<Vec<(i64, String)>>,
    ) {
        if *current.borrow() != items {
            let mut names = vec![first.to_string()];
            names.extend(items.iter().map(|x| x.1.clone()));
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            dd.set_model(Some(&gtk::StringList::new(&refs)));
            *current.borrow_mut() = items.to_vec();
        }
    }

    pub fn apply(&self, v: &SsView) {
        self.flag.set(true);
        self.session.set(v.session_id);
        self.name.set_text(&v.name);
        self.sync_drop(
            &self.persona,
            &tr("ss-persona-default"),
            &v.personas,
            &self.personas,
        );
        let models: Vec<(i64, String)> = v
            .models
            .iter()
            .map(|m| (m.account_id, m.label.clone()))
            .collect();
        self.sync_drop(&self.model, &tr("ss-model-default"), &models, &self.models);
        let idx = |list: &RefCell<Vec<(i64, String)>>, id: Option<i64>| {
            id.and_then(|id| list.borrow().iter().position(|x| x.0 == id))
                .map_or(0, |i| i as u32 + 1)
        };
        self.persona.set_selected(idx(&self.personas, v.persona));
        self.model.set_selected(idx(&self.models, v.model));
        if self.path.text() != v.path {
            self.path.set_text(&v.path);
        }
        self.ctx_row.set_visible(v.is_thread);
        self.ctx.set_active(v.inherit_ctx);
        self.mem.set_active(v.mem);
        self.auto.set_active(v.auto);
        self.notif.set_selected(match v.notif {
            NotifMode::All => 0,
            NotifMode::Important => 1,
            NotifMode::Off => 2,
        });
        self.new_thread
            .set_sensitive(!v.is_thread && v.session_id.is_some());
        self.perms.apply(&v.perms);
        self.tasks.apply(v);
        self.notice.set_text(v.notice.as_deref().unwrap_or(""));
        self.exported.set_text(v.exported.as_deref().unwrap_or(""));
        match &v.confirm {
            Some(c) => {
                self.confirm_title.set_text(&c.title);
                while let Some(ch) = self.confirm_lines.first_child() {
                    self.confirm_lines.remove(&ch);
                }
                for line in &c.lines {
                    self.confirm_lines.append(&label(line));
                }
                self.confirm.set_visible(true);
            }
            None => self.confirm.set_visible(false),
        }
        self.flag.set(false);
    }

    /// Confirmation is shown (for UI scenarios).
    pub fn confirm_visible(&self) -> bool {
        self.confirm.is_visible()
    }

    pub fn confirm_text(&self) -> Vec<String> {
        let mut out = vec![self.confirm_title.text().to_string()];
        let mut child = self.confirm_lines.first_child();
        while let Some(c) = child {
            if let Some(l) = c.downcast_ref::<gtk::Label>() {
                out.push(l.text().to_string());
            }
            child = c.next_sibling();
        }
        out
    }
}
