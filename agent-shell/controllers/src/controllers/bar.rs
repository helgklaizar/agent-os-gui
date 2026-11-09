//! `BarController`: clock, date, weather and the Agent OS menu.

use crate::time::Instant;

use crate::bar_types::WeatherFrame;
use crate::bus::BusEvent;
use crate::clock_fmt::{format_date, format_time};
use crate::controller::{Controller, Effect};
use crate::i18n::{tr, tr_args};
use crate::level::{Level, round_pct};
use crate::timezone::Zone;
use crate::types::{ConnState, DialogKind, FocusTarget, MenuItem, SettingsPage};

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherView {
    /// "24°" or "—".
    pub temp: String,
    /// GTK theme icon name for the state.
    pub icon: &'static str,
    pub text: String,
    pub place: String,
    pub tooltip: String,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub item: MenuItem,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BarView {
    /// `HH:MM:SS`.
    pub time: String,
    /// "Mon, Nov 2".
    pub date: String,
    pub weather: WeatherView,
    pub menu_open: bool,
    pub menu: Vec<MenuEntry>,
    /// "No connection" banner (WS dropped).
    pub offline: bool,
    /// A separate "The core does not start" banner (`core_unavailable`).
    pub core_unavailable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BarMsg {
    /// Clock tick (unix seconds from the controllable clock).
    Tick(i64),
    /// `None` means a frame without data.
    Weather(Option<WeatherFrame>),
    MenuToggle,
    /// Closing the menu by clicking outside or pressing Esc.
    MenuDismiss,
    MenuSelect(MenuItem),
    Connection(ConnState),
}

pub struct BarController {
    zone: Zone,
    unix: Option<i64>,
    weather: Option<WeatherFrame>,
    menu_open: bool,
    offline: bool,
    core_unavailable: bool,
}

/// GTK theme icon for the state from the frame; an unknown state means "no icon".
fn weather_icon(name: &str) -> &'static str {
    match name {
        "sun" => "weather-clear-symbolic",
        "moon" => "weather-clear-night-symbolic",
        "partly" => "weather-few-clouds-symbolic",
        "cloud" => "weather-overcast-symbolic",
        "rain" => "weather-showers-symbolic",
        "storm" => "weather-storm-symbolic",
        "snow" => "weather-snow-symbolic",
        "fog" => "weather-fog-symbolic",
        _ => NO_ICON,
    }
}

const NO_ICON: &str = "weather-none-available-symbolic";

impl BarController {
    pub fn new(zone: Zone) -> Self {
        Self {
            zone,
            unix: None,
            weather: None,
            menu_open: false,
            offline: false,
            core_unavailable: false,
        }
    }

    fn weather_view(&self) -> WeatherView {
        let live = self
            .weather
            .as_ref()
            .filter(|w| w.temp_c.is_some() && !self.offline);
        let Some(w) = live else {
            return WeatherView {
                temp: tr("value-none"),
                icon: NO_ICON,
                text: String::new(),
                place: String::new(),
                tooltip: tr("bar-no-data"),
                level: Level::NoData,
            };
        };
        let temp = w.temp_c.map_or_else(
            || tr("value-none"),
            |t| tr_args("metric-value-degrees", &[("v", &round_pct(t).to_string())]),
        );
        let (text, place) = (
            w.text.clone().unwrap_or_default(),
            w.place.clone().unwrap_or_default(),
        );
        let parts: Vec<&str> = [text.as_str(), place.as_str()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        let tooltip = if parts.is_empty() {
            temp.clone()
        } else {
            parts.join(", ")
        };
        WeatherView {
            temp,
            icon: w.icon.as_deref().map_or(NO_ICON, weather_icon),
            text,
            place,
            tooltip,
            level: Level::Neutral,
        }
    }

    fn select(&mut self, item: MenuItem) -> Vec<Effect> {
        self.menu_open = false;
        let event = match item {
            MenuItem::About => BusEvent::OpenDialog {
                kind: DialogKind::About,
                opener: FocusTarget::MenuItem(MenuItem::About),
            },
            MenuItem::User => BusEvent::OpenSettings(SettingsPage::Account),
            MenuItem::Settings => BusEvent::OpenSettings(SettingsPage::Root),
            MenuItem::Logout => BusEvent::OpenDialog {
                kind: DialogKind::ConfirmLogout,
                opener: FocusTarget::MenuItem(MenuItem::Logout),
            },
        };
        vec![Effect::Publish(event)]
    }
}

impl Controller for BarController {
    type Msg = BarMsg;
    type View = BarView;

    fn update(&mut self, msg: BarMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            BarMsg::Tick(unix) => self.unix = Some(unix),
            BarMsg::Weather(w) => self.weather = w,
            BarMsg::MenuToggle => self.menu_open = !self.menu_open,
            BarMsg::MenuDismiss => self.menu_open = false,
            BarMsg::MenuSelect(item) => return self.select(item),
            BarMsg::Connection(state) => {
                self.offline = matches!(
                    state,
                    ConnState::Reconnecting | ConnState::Offline | ConnState::CoreUnavailable
                );
                self.core_unavailable = state == ConnState::CoreUnavailable;
            }
        }
        Vec::new()
    }

    fn view(&self) -> BarView {
        let (time, date) = match self.unix {
            Some(u) => {
                let off = self.zone.offset_at(u);
                (format_time(u, off), format_date(u, off))
            }
            None => (tr("value-none"), tr("value-none")),
        };
        let entry = |item, label: String| MenuEntry { item, label };
        BarView {
            time,
            date,
            weather: self.weather_view(),
            menu_open: self.menu_open,
            menu: vec![
                entry(MenuItem::About, tr("menu-about")),
                entry(MenuItem::User, tr("menu-user")),
                entry(MenuItem::Settings, tr("menu-settings")),
                entry(MenuItem::Logout, tr("menu-logout")),
            ],
            offline: self.offline,
            core_unavailable: self.core_unavailable,
        }
    }
}
