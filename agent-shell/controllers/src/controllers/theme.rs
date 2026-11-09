//! `ThemeController`: auto / dark / light; the core wins over the cache.

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::controller::{Controller, CoreCommand, Effect};
use crate::prefs::UiPrefs;
use crate::types::{ResolvedTheme, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMsg {
    /// System theme changed (`true` means dark).
    SystemChanged { dark: bool },
    /// The user picked a theme.
    UserSelected(Theme),
    /// Value from the `ui.json` cache at start.
    CacheLoaded(Theme),
    /// Value from the core (`settings.theme`) after connecting or after a `412`.
    CoreValue(Theme),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeView {
    pub selected: Theme,
    pub resolved: ResolvedTheme,
}

pub struct ThemeController {
    system_dark: bool,
    selected: Theme,
}

impl ThemeController {
    /// `system_dark` is the current system theme. Until the cache and the core reply, `Auto` is selected.
    pub fn new(system_dark: bool) -> Self {
        Self {
            system_dark,
            selected: Theme::Auto,
        }
    }

    fn resolved(&self) -> ResolvedTheme {
        match self.selected {
            Theme::Dark => ResolvedTheme::Dark,
            Theme::Light => ResolvedTheme::Light,
            Theme::Auto if self.system_dark => ResolvedTheme::Dark,
            Theme::Auto => ResolvedTheme::Light,
        }
    }

    fn changed(&self) -> Effect {
        Effect::Publish(BusEvent::ThemeChanged(self.selected))
    }
}

impl Controller for ThemeController {
    type Msg = ThemeMsg;
    type View = ThemeView;

    fn update(&mut self, msg: ThemeMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            ThemeMsg::SystemChanged { dark } => {
                let before = self.resolved();
                self.system_dark = dark;
                if self.resolved() == before {
                    return Vec::new();
                }
                vec![self.changed()]
            }
            ThemeMsg::UserSelected(theme) => {
                if theme == self.selected {
                    return Vec::new();
                }
                self.selected = theme;
                vec![
                    self.changed(),
                    Effect::Command(CoreCommand::PatchTheme(theme)),
                    Effect::Persist(UiPrefs {
                        theme: Some(theme),
                        ..UiPrefs::default()
                    }),
                ]
            }
            // the cache is already in `ui.json`: nothing to rewrite or send to the core
            ThemeMsg::CacheLoaded(theme) => {
                if theme == self.selected {
                    return Vec::new();
                }
                self.selected = theme;
                vec![self.changed()]
            }
            // the core is the source of truth: apply and update the cache, do not send back to the core
            ThemeMsg::CoreValue(theme) => {
                if theme == self.selected {
                    return Vec::new();
                }
                self.selected = theme;
                vec![
                    self.changed(),
                    Effect::Persist(UiPrefs {
                        theme: Some(theme),
                        ..UiPrefs::default()
                    }),
                ]
            }
        }
    }

    fn view(&self) -> ThemeView {
        ThemeView {
            selected: self.selected,
            resolved: self.resolved(),
        }
    }
}
