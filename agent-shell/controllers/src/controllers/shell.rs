//! `ShellController`: column layout, current session, toast.

use crate::time::Instant;
use std::time::Duration;

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::layout::{LayoutState, resolve};
use crate::prefs::{UiFile, UiPrefs, WindowPrefs};
use crate::types::{Column, Manual};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub session_id: i64,
    pub text: String,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMsg {
    WindowResized { w: u32, h: u32 },
    SetManual(Column, Manual),
    ToggleColumn(Column),
    SessionClosed,
    Tick,
    Bus(BusEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellView {
    pub layout: LayoutState,
    pub current_session: Option<i64>,
    pub tools_visible: bool,
    pub toast: Option<Toast>,
}

/// How long a toast is shown.
pub const TOAST_DURATION: Duration = Duration::from_secs(4);

pub struct ShellController {
    prefs: UiFile,
    view: ShellView,
}

impl ShellController {
    /// `prefs` is the parsed `ui.json` (or defaults): manual column states and window size.
    pub fn new(prefs: UiFile) -> Self {
        let view = ShellView {
            layout: resolve(prefs.window.w, &prefs.columns),
            current_session: None,
            tools_visible: false,
            toast: None,
        };
        Self { prefs, view }
    }

    /// Recompute the layout; `LayoutChanged` is published only if it changed.
    fn relayout(&mut self, fx: &mut Vec<Effect>) {
        let layout = resolve(self.prefs.window.w, &self.prefs.columns);
        if layout != self.view.layout {
            self.view.layout = layout;
            fx.push(Effect::Publish(BusEvent::LayoutChanged));
        }
    }

    fn set_manual(&mut self, column: Column, manual: Manual, fx: &mut Vec<Effect>) {
        let cols = &mut self.prefs.columns;
        match column {
            Column::Notifications => cols.notifications.manual = manual,
            Column::Sessions => cols.sessions.manual = manual,
            Column::Tools => cols.tools.manual = manual,
        }
        fx.push(Effect::Persist(UiPrefs {
            columns: Some(self.prefs.columns),
            ..UiPrefs::default()
        }));
        self.relayout(fx);
    }

    fn is_collapsed(&self, column: Column) -> bool {
        let l = &self.view.layout;
        match column {
            Column::Notifications => l.notifications_collapsed,
            Column::Sessions => l.sessions_collapsed,
            Column::Tools => l.tools_collapsed,
        }
    }
}

impl Controller for ShellController {
    type Msg = ShellMsg;
    type View = ShellView;

    fn update(&mut self, msg: ShellMsg, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match msg {
            ShellMsg::WindowResized { w, h } => {
                let window = WindowPrefs { w, h };
                if window != self.prefs.window {
                    self.prefs.window = window;
                    fx.push(Effect::Persist(UiPrefs {
                        window: Some(window),
                        ..UiPrefs::default()
                    }));
                    self.relayout(&mut fx);
                }
            }
            ShellMsg::SetManual(column, manual) => self.set_manual(column, manual, &mut fx),
            ShellMsg::ToggleColumn(column) => {
                let manual = if self.is_collapsed(column) {
                    Manual::Expanded
                } else {
                    Manual::Collapsed
                };
                self.set_manual(column, manual, &mut fx);
            }
            ShellMsg::SessionClosed => {
                self.view.current_session = None;
                self.view.tools_visible = false;
            }
            ShellMsg::Tick => {
                if self
                    .view
                    .toast
                    .as_ref()
                    .is_some_and(|t| now >= t.expires_at)
                {
                    self.view.toast = None;
                }
            }
            ShellMsg::Bus(BusEvent::OpenSession { session_id, .. }) => {
                self.view.current_session = Some(session_id);
                self.view.tools_visible = true;
            }
            ShellMsg::Bus(BusEvent::ShowToast { session_id, text }) => {
                self.view.toast = Some(Toast {
                    session_id,
                    text,
                    expires_at: now + TOAST_DURATION,
                });
            }
            ShellMsg::Bus(BusEvent::ExpandColumn(column)) => {
                self.set_manual(column, Manual::Expanded, &mut fx);
            }
            // other bus events do not concern the shell
            ShellMsg::Bus(_) => {}
        }
        fx
    }

    fn view(&self) -> ShellView {
        self.view.clone()
    }
}
