//! `ToolsController`: the tools column. Three tabs (browser, terminal, session settings), visible only when
//! a session is open; the selected tab is kept when the session changes; selecting in a collapsed column expands it.

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::i18n::tr;
use crate::types::{Column, Tab};

/// Tools column tabs in order.
pub const TOOL_TABS: [Tab; 3] = [Tab::Browser, Tab::Terminal, Tab::SessionSettings];

#[derive(Debug, Clone, PartialEq)]
pub enum ToolsMsg {
    SessionOpened(i64),
    SessionClosed,
    Select(Tab),
    /// Column collapsed or expanded (decided by the layout).
    Collapsed(bool),
    Bus(BusEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTab {
    pub tab: Tab,
    /// Name for the accessibility tree and tooltip.
    pub name: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsView {
    pub visible: bool,
    pub active: Tab,
    pub collapsed: bool,
    pub session: Option<i64>,
    pub tabs: Vec<ToolTab>,
}

#[derive(Debug)]
pub struct ToolsController {
    session: Option<i64>,
    active: Tab,
    collapsed: bool,
}

impl Default for ToolsController {
    fn default() -> Self {
        ToolsController {
            session: None,
            active: Tab::Browser,
            collapsed: false,
        }
    }
}

impl ToolsController {
    pub fn new() -> Self {
        Self::default()
    }

    fn select(&mut self, tab: Tab, fx: &mut Vec<Effect>) {
        if !TOOL_TABS.contains(&tab) {
            return;
        }
        self.active = tab;
        if self.collapsed {
            fx.push(Effect::Publish(BusEvent::ExpandColumn(Column::Tools)));
        }
    }
}

fn tab_name(tab: Tab) -> String {
    match tab {
        Tab::Browser => tr("tools-tab-browser"),
        Tab::Terminal => tr("tools-tab-terminal"),
        _ => tr("tools-tab-settings"),
    }
}

impl Controller for ToolsController {
    type Msg = ToolsMsg;
    type View = ToolsView;

    fn update(&mut self, msg: ToolsMsg, _now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match msg {
            ToolsMsg::SessionOpened(id) => self.session = Some(id),
            ToolsMsg::SessionClosed => self.session = None,
            ToolsMsg::Select(tab) => self.select(tab, &mut fx),
            ToolsMsg::Collapsed(c) => self.collapsed = c,
            ToolsMsg::Bus(BusEvent::OpenSession { session_id, tab }) => {
                self.session = Some(session_id);
                // chat and module tabs do not belong to this column: the selection stays; no need to expand the column from the feed
                if TOOL_TABS.contains(&tab) {
                    self.active = tab;
                }
            }
            ToolsMsg::Bus(_) => {}
        }
        fx
    }

    fn view(&self) -> ToolsView {
        ToolsView {
            visible: self.session.is_some(),
            active: self.active,
            collapsed: self.collapsed,
            session: self.session,
            tabs: TOOL_TABS
                .iter()
                .map(|t| ToolTab {
                    tab: *t,
                    name: tab_name(*t),
                    selected: *t == self.active,
                })
                .collect(),
        }
    }
}
