//! `SettingsController`: the Settings dialog: group navigation, a page with a draft, saving through the core.
//! There is no sub-controller per page: pages are described by tables of fields and actions (`settings_*`), the state
//! is shared, so the draft is not lost when switching pages. The core is the only source of data.

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::i18n::tr;
use crate::permissions::{Mode, persona_rule_options};
use crate::settings_actions::{act, confirmed};
use crate::settings_fields::parse;
use crate::settings_pages::{rows, spec_by_id};
use crate::settings_perms::saved_perm;
use crate::settings_reply::reply;
use crate::settings_save::{cmd, current_mode, save};
use crate::settings_state::State;
use crate::settings_types::{Close, Cmd, Group, NAV, resolve};
use crate::settings_view::{Confirm, NavGroup, NavItem, SettingsMsg, SettingsView};
use crate::time::Instant;
use crate::types::{FocusTarget, MenuItem, SettingsPage, Theme};

#[derive(Default)]
pub struct SettingsController {
    st: State,
}

fn page_label(page: SettingsPage) -> String {
    match page {
        SettingsPage::Account | SettingsPage::Root => tr("set-page-account"),
        SettingsPage::About => tr("set-page-about"),
        SettingsPage::Personas => tr("set-page-personas"),
        SettingsPage::Models => tr("set-page-models"),
        SettingsPage::Connections => tr("set-page-connections"),
        SettingsPage::Permissions => tr("set-page-permissions"),
        SettingsPage::Security => tr("set-page-security"),
        SettingsPage::Schedule => tr("set-page-schedule"),
        SettingsPage::Modules => tr("set-page-modules"),
        SettingsPage::Tunnels => tr("set-page-tunnels"),
        SettingsPage::Files => tr("set-page-files"),
        SettingsPage::Indicators => tr("set-page-indicators"),
    }
}

fn group_label(g: Group) -> String {
    match g {
        Group::General => tr("set-group-general"),
        Group::Ai => tr("set-group-ai"),
        Group::System => tr("set-group-system"),
    }
}

fn theme_of(value: &str) -> Option<Theme> {
    match value {
        "auto" => Some(Theme::Auto),
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

impl SettingsController {
    pub fn new() -> SettingsController {
        SettingsController::default()
    }

    /// Core data visible to pages (for consistency checks and field parsing).
    pub fn model(&self) -> &crate::settings_model::CoreModel {
        &self.st.model
    }

    fn open(&mut self, page: SettingsPage, opener: Option<FocusTarget>) -> Vec<Effect> {
        let first = !self.st.open;
        self.st.open = true;
        self.st.page = Some(resolve(page));
        self.st.confirm = None;
        self.st.banner = None;
        if first {
            self.st.opener = opener;
            return vec![cmd(Cmd::FetchAll)];
        }
        Vec::new()
    }

    fn close(&mut self) -> Vec<Effect> {
        if self.st.confirm.take().is_some() || !self.st.open {
            return Vec::new();
        }
        self.st.open = false;
        self.st.banner = None;
        match self.st.opener.take() {
            Some(target) => vec![Effect::Publish(BusEvent::FocusRestore(target))],
            None => Vec::new(),
        }
    }

    fn edit_perm(&mut self, id: &str, value: &str) {
        let Some(mode) = Mode::parse(value) else {
            return;
        };
        let mut parts = id.splitn(3, ':');
        let (_, level, action) = (
            parts.next(),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        let persona = self.st.perm_persona.or_else(|| {
            self.st
                .model
                .personas
                .first()
                .and_then(crate::settings_model::id_of)
        });
        let scope_id = if level == "global" {
            0
        } else {
            persona.unwrap_or(0)
        };
        // a lower level only tightens: a value weaker than the upper level is not accepted
        if level != "global" {
            let global = current_mode(&self.st, "global", 0, action);
            if !persona_rule_options(Some(global)).contains(&mode) {
                self.st.errors.insert(id.to_string(), tr("set-err-loosen"));
                return;
            }
        }
        self.st.errors.remove(id);
        let saved = saved_perm(&self.st, level, scope_id, action);
        if saved == Some(mode) {
            self.st.text.remove(id);
        } else {
            self.st
                .text
                .insert(id.to_string(), mode.as_str().to_string());
        }
    }

    fn edit(&mut self, id: &str, value: String) {
        if id == "theme" {
            return;
        }
        if id.starts_with("perm:") {
            self.edit_perm(id, &value);
            return;
        }
        let Some(spec) = spec_by_id(&self.st, id) else {
            return;
        };
        self.st.edit(&spec, value.clone());
        if let Err(bad) = parse(&spec, &value) {
            self.st.errors.insert(id.to_string(), bad.text());
        }
    }
}

impl Controller for SettingsController {
    type Msg = SettingsMsg;
    type View = SettingsView;

    fn update(&mut self, msg: SettingsMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            SettingsMsg::Bus(BusEvent::OpenSettings(page)) => {
                let opener = match page {
                    SettingsPage::Root => Some(FocusTarget::MenuItem(MenuItem::Settings)),
                    SettingsPage::Account => Some(FocusTarget::MenuItem(MenuItem::User)),
                    _ => None,
                };
                self.open(page, opener)
            }
            SettingsMsg::Bus(BusEvent::ThemeChanged(theme)) | SettingsMsg::ThemeNow(theme) => {
                self.st.theme = theme;
                Vec::new()
            }
            SettingsMsg::Bus(_) => Vec::new(),
            SettingsMsg::Open(page) => self.open(page, None),
            SettingsMsg::Close(Close::Esc | Close::Button | Close::Backdrop) => self.close(),
            SettingsMsg::Select(page) => {
                self.st.page = Some(resolve(page));
                self.st.banner = None;
                Vec::new()
            }
            SettingsMsg::Edit { id, value } if id == "theme" => match theme_of(&value) {
                Some(theme) => {
                    self.st.theme = theme;
                    vec![Effect::Publish(BusEvent::SelectTheme(theme))]
                }
                None => Vec::new(),
            },
            SettingsMsg::Edit { id, value } => {
                self.edit(&id, value);
                Vec::new()
            }
            SettingsMsg::EditSecret { id, value } => {
                if value.is_empty() {
                    self.st.secrets.remove(&id);
                } else if id.starts_with("s:") {
                    self.st.errors.remove(&id);
                    self.st.secrets.insert(id, value);
                }
                Vec::new()
            }
            SettingsMsg::Save => save(&mut self.st),
            SettingsMsg::Act(action) => act(&mut self.st, action),
            SettingsMsg::Confirm(yes) => confirmed(&mut self.st, yes),
            SettingsMsg::Reply(r) => reply(&mut self.st, r),
            SettingsMsg::Changed { page, rev } => {
                if self.st.model.docs.get(&page).is_some_and(|d| d.rev != rev) {
                    vec![cmd(Cmd::FetchPage(page))]
                } else {
                    Vec::new()
                }
            }
            SettingsMsg::Quotas(q) => {
                self.st.model.quotas = q;
                Vec::new()
            }
            SettingsMsg::Sys(s) => {
                self.st.model.sys = s;
                Vec::new()
            }
            SettingsMsg::Env { system, config } => {
                self.st.system = system;
                self.st.config = config;
                Vec::new()
            }
        }
    }

    fn view(&self) -> SettingsView {
        let page = self.st.page();
        let nav = NAV
            .iter()
            .map(|(g, pages)| NavGroup {
                title: group_label(*g),
                items: pages
                    .iter()
                    .map(|p| NavItem {
                        page: *p,
                        label: page_label(*p),
                        selected: *p == page,
                    })
                    .collect(),
            })
            .collect();
        SettingsView {
            open: self.st.open,
            page,
            title: page_label(page),
            nav,
            rows: if self.st.open {
                rows(&self.st)
            } else {
                Vec::new()
            },
            banner: self.st.banner.clone(),
            confirm: self.st.confirm.as_ref().map(|c| Confirm {
                title: c.title.clone(),
                lines: c.lines.clone(),
            }),
            dirty: self.st.is_dirty(),
            saving: self.st.pending > 0,
            loading: !self.st.model.loaded,
            save_visible: true,
            close_visible: true,
        }
    }
}
