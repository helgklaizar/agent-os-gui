//! S6-03: tools column: visibility, tabs, tab kept on session change, expanding a collapsed column.

use std::time::Instant;

use controllers::controllers::tools::{TOOL_TABS, ToolsController, ToolsMsg};
use controllers::{BusEvent, Column, Controller, Effect, Tab};

fn send(c: &mut ToolsController, m: ToolsMsg) -> Vec<Effect> {
    c.update(m, Instant::now())
}

#[test]
fn tools_hidden_without_session_and_shown_with_one() {
    let mut c = ToolsController::new();
    assert!(!c.view().visible, "without a session the column is hidden");
    send(&mut c, ToolsMsg::SessionOpened(3));
    assert!(c.view().visible);
    assert_eq!(c.view().session, Some(3));
    send(&mut c, ToolsMsg::SessionClosed);
    assert!(!c.view().visible);
    assert_eq!(c.view().session, None);
}

#[test]
fn tools_three_tabs_in_order_with_names() {
    let c = ToolsController::new();
    let v = c.view();
    assert_eq!(
        v.tabs.iter().map(|t| t.tab).collect::<Vec<_>>(),
        TOOL_TABS.to_vec()
    );
    assert_eq!(
        TOOL_TABS,
        [Tab::Browser, Tab::Terminal, Tab::SessionSettings]
    );
    assert_eq!(
        v.tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Browser", "Terminal", "Session settings"]
    );
    for t in &v.tabs {
        assert!(
            !t.name.is_empty() && !t.name.contains('⟦'),
            "tab name {:?}",
            t.tab
        );
    }
    assert_eq!(v.tabs.iter().filter(|t| t.selected).count(), 1);
    assert_eq!(v.active, Tab::Browser, "browser by default");
}

#[test]
fn tools_active_tab_survives_session_switch() {
    let mut c = ToolsController::new();
    send(&mut c, ToolsMsg::SessionOpened(1));
    send(&mut c, ToolsMsg::Select(Tab::Terminal));
    assert_eq!(c.view().active, Tab::Terminal);
    send(&mut c, ToolsMsg::SessionOpened(2));
    assert_eq!(
        c.view().active,
        Tab::Terminal,
        "a session change does not reset the tab"
    );
    send(&mut c, ToolsMsg::SessionClosed);
    send(&mut c, ToolsMsg::SessionOpened(3));
    assert_eq!(
        c.view().active,
        Tab::Terminal,
        "nor does closing the session"
    );
    let selected: Vec<Tab> = c
        .view()
        .tabs
        .iter()
        .filter(|t| t.selected)
        .map(|t| t.tab)
        .collect();
    assert_eq!(selected, vec![Tab::Terminal]);
}

#[test]
fn tools_select_in_collapsed_column_expands_it() {
    let mut c = ToolsController::new();
    send(&mut c, ToolsMsg::SessionOpened(1));
    send(&mut c, ToolsMsg::Collapsed(true));
    let fx = send(&mut c, ToolsMsg::Select(Tab::SessionSettings));
    assert_eq!(c.view().active, Tab::SessionSettings);
    assert!(
        fx.contains(&Effect::Publish(BusEvent::ExpandColumn(Column::Tools))),
        "selecting a tab expands the column: {fx:?}"
    );
    let open = send(&mut c, ToolsMsg::Collapsed(false));
    assert!(open.is_empty());
    let fx = send(&mut c, ToolsMsg::Select(Tab::Browser));
    assert!(
        fx.is_empty(),
        "nothing to expand in an expanded column: {fx:?}"
    );
}

#[test]
fn tools_select_other_tabs_is_ignored() {
    let mut c = ToolsController::new();
    send(&mut c, ToolsMsg::SessionOpened(1));
    send(&mut c, ToolsMsg::Select(Tab::Chat));
    send(&mut c, ToolsMsg::Select(Tab::Modules));
    assert_eq!(
        c.view().active,
        Tab::Browser,
        "chat and modules are not tabs of this column"
    );
}

#[test]
fn tools_open_session_event_selects_requested_tool_tab() {
    let mut c = ToolsController::new();
    send(
        &mut c,
        ToolsMsg::Bus(BusEvent::OpenSession {
            session_id: 9,
            tab: Tab::Terminal,
        }),
    );
    assert!(c.view().visible);
    assert_eq!(c.view().session, Some(9));
    assert_eq!(
        c.view().active,
        Tab::Terminal,
        "the notification button leads to a tab"
    );
    send(
        &mut c,
        ToolsMsg::Bus(BusEvent::OpenSession {
            session_id: 10,
            tab: Tab::Chat,
        }),
    );
    assert_eq!(
        c.view().active,
        Tab::Terminal,
        "chat is not a tools tab, the tab stays"
    );
    assert_eq!(c.view().session, Some(10));
}
