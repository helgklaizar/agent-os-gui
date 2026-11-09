//! Signals of the "Session settings" tab widgets: widget value -> message to the controller.
//! While the view itself switches widgets (`flag`), signals stay silent.

use std::rc::Rc;

use controllers::ss_view::{NotifMode, SsMsg};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::ss_pane::SettingsPane;

impl SettingsPane {
    pub(crate) fn wire(
        self: &Rc<Self>,
        send: &Rc<dyn Fn(SsMsg)>,
        on_move: &Rc<dyn Fn(i64)>,
        pick: &gtk::Button,
        yes: &gtk::Button,
        no: &gtk::Button,
    ) {
        let weak = Rc::downgrade(self);
        let guarded = |f: Rc<dyn Fn(&SettingsPane)>| {
            let weak = weak.clone();
            move || {
                if let Some(p) = weak.upgrade()
                    && !p.flag.get()
                {
                    f(&p);
                }
            }
        };
        let s = send.clone();
        let on_persona = guarded(Rc::new(move |p: &SettingsPane| {
            let i = p.persona.selected() as usize;
            let id = (i > 0)
                .then(|| p.personas.borrow().get(i - 1).map(|x| x.0))
                .flatten();
            s(SsMsg::SetPersona(id));
        }));
        self.persona.connect_selected_notify(move |_| on_persona());
        let s = send.clone();
        let on_path = guarded(Rc::new(move |p: &SettingsPane| {
            s(SsMsg::SetPath(p.path.text().to_string()))
        }));
        self.path.connect_activate(move |_| on_path());
        let s = send.clone();
        let on_ctx = guarded(Rc::new(move |p: &SettingsPane| {
            s(SsMsg::SetCtxInherit(p.ctx.is_active()))
        }));
        self.ctx.connect_active_notify(move |_| on_ctx());
        let s = send.clone();
        let on_model = guarded(Rc::new(move |p: &SettingsPane| {
            let i = p.model.selected() as usize;
            let id = (i > 0)
                .then(|| p.models.borrow().get(i - 1).map(|x| x.0))
                .flatten();
            s(SsMsg::SetModel(id));
        }));
        self.model.connect_selected_notify(move |_| on_model());
        let s = send.clone();
        let on_mem = guarded(Rc::new(move |p: &SettingsPane| {
            s(SsMsg::SetMem(p.mem.is_active()))
        }));
        self.mem.connect_active_notify(move |_| on_mem());
        let s = send.clone();
        let on_auto = guarded(Rc::new(move |p: &SettingsPane| {
            s(SsMsg::SetAuto(p.auto.is_active()))
        }));
        self.auto.connect_active_notify(move |_| on_auto());
        let s = send.clone();
        let on_notif = guarded(Rc::new(move |p: &SettingsPane| {
            s(SsMsg::SetNotif(match p.notif.selected() {
                1 => NotifMode::Important,
                2 => NotifMode::Off,
                _ => NotifMode::All,
            }));
        }));
        self.notif.connect_selected_notify(move |_| on_notif());
        for (b, msg) in [
            (&self.new_thread, SsMsg::NewThread),
            (&self.clear, SsMsg::AskClear),
            (&self.delete, SsMsg::AskDelete),
            (&self.export, SsMsg::Export),
            (yes, SsMsg::Confirm),
            (no, SsMsg::Cancel),
        ] {
            let s = send.clone();
            b.connect_clicked(move |_| s(msg.clone()));
        }
        let weak = Rc::downgrade(self);
        let on_move = on_move.clone();
        self.move_to.connect_clicked(move |_| {
            if let Some(id) = weak.upgrade().and_then(|p| p.session.get()) {
                on_move(id);
            }
        });
        let (s, weak) = (send.clone(), Rc::downgrade(self));
        pick.connect_clicked(move |b| {
            let (Some(window), Some(p)) = (b.root().and_downcast::<gtk::Window>(), weak.upgrade())
            else {
                return;
            };
            let (s, weak) = (s.clone(), Rc::downgrade(&p));
            gtk::FileDialog::new().select_folder(
                Some(&window),
                None::<&gtk::gio::Cancellable>,
                move |res| {
                    if let (Ok(file), Some(_)) = (res, weak.upgrade())
                        && let Some(path) = file.path()
                    {
                        s(SsMsg::SetPath(path.to_string_lossy().into_owned()));
                    }
                },
            );
        });
    }
}
