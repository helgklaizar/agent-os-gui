//! Tests of `ShellController`: session, layout, toast.

use std::time::{Duration, Instant};

use controllers::controllers::shell::{ShellController, ShellMsg};
use controllers::prefs::{UiFile, UiPrefs};
use controllers::{BusEvent, Column, Controller, Effect, Manual, Tab};

fn wide() -> UiFile {
    let mut f = UiFile::default();
    f.window.w = 2000;
    f
}

fn open(c: &mut ShellController, id: i64, now: Instant) -> Vec<Effect> {
    c.update(
        ShellMsg::Bus(BusEvent::OpenSession {
            session_id: id,
            tab: Tab::Browser,
        }),
        now,
    )
}

#[test]
fn shell_start_without_session_hides_tools() {
    let c = ShellController::new(wide());
    let v = c.view();
    assert_eq!(v.current_session, None);
    assert!(!v.tools_visible);
}

#[test]
fn shell_open_session_shows_tools_and_close_hides() {
    let now = Instant::now();
    let mut c = ShellController::new(wide());
    open(&mut c, 7, now);
    assert_eq!(c.view().current_session, Some(7));
    assert!(c.view().tools_visible);
    open(&mut c, 9, now);
    assert_eq!(c.view().current_session, Some(9));
    c.update(ShellMsg::SessionClosed, now);
    assert_eq!(c.view().current_session, None);
    assert!(!c.view().tools_visible);
}

#[test]
fn shell_toggle_column_sets_manual_persists_and_announces() {
    let now = Instant::now();
    let mut c = ShellController::new(wide());
    assert!(!c.view().layout.tools_collapsed);

    let fx = c.update(ShellMsg::ToggleColumn(Column::Tools), now);
    assert!(c.view().layout.tools_collapsed);
    assert_eq!(c.view().layout.tools_w, 56);
    assert!(fx.contains(&Effect::Publish(BusEvent::LayoutChanged)));
    let saved = fx.iter().find_map(|e| match e {
        Effect::Persist(UiPrefs {
            columns: Some(cols),
            ..
        }) => Some(*cols),
        _ => None,
    });
    assert_eq!(saved.map(|c| c.tools.manual), Some(Manual::Collapsed));

    c.update(ShellMsg::ToggleColumn(Column::Tools), now);
    assert!(!c.view().layout.tools_collapsed);
}

#[test]
fn shell_toggle_expands_auto_collapsed_column_manually() {
    let now = Instant::now();
    let mut f = UiFile::default();
    f.window.w = 1024;
    let mut c = ShellController::new(f);
    assert!(c.view().layout.tools_collapsed, "auto: narrow window");
    let fx = c.update(ShellMsg::ToggleColumn(Column::Tools), now);
    assert!(!c.view().layout.tools_collapsed);
    let saved = fx.iter().find_map(|e| match e {
        Effect::Persist(UiPrefs {
            columns: Some(cols),
            ..
        }) => Some(cols.tools.manual),
        _ => None,
    });
    assert_eq!(saved, Some(Manual::Expanded));
}

#[test]
fn shell_restores_manual_state_from_prefs() {
    let mut f = wide();
    f.columns.notifications.manual = Manual::Collapsed;
    let c = ShellController::new(f);
    assert!(c.view().layout.notifications_collapsed);
}

#[test]
fn shell_resize_recomputes_layout_and_persists_window() {
    let now = Instant::now();
    let mut c = ShellController::new(wide());
    let fx = c.update(ShellMsg::WindowResized { w: 1024, h: 640 }, now);
    let l = c.view().layout;
    assert!(l.notifications_collapsed && l.tools_collapsed && !l.sessions_collapsed);
    assert_eq!(l.chat_w, 676);
    assert!(fx.contains(&Effect::Publish(BusEvent::LayoutChanged)));
    assert!(fx.iter().any(|e| matches!(e, Effect::Persist(UiPrefs { window: Some(w), .. }) if w.w == 1024 && w.h == 640)));
}

#[test]
fn shell_resize_without_layout_change_does_not_announce() {
    let now = Instant::now();
    let mut c = ShellController::new(wide());
    let fx = c.update(ShellMsg::WindowResized { w: 2000, h: 700 }, now);
    assert!(!fx.contains(&Effect::Publish(BusEvent::LayoutChanged)));
}

#[test]
fn shell_toast_shows_for_4s() {
    let t0 = Instant::now();
    let mut c = ShellController::new(wide());
    c.update(
        ShellMsg::Bus(BusEvent::ShowToast {
            session_id: 3,
            text: "<b>done</b>".into(),
        }),
        t0,
    );
    let toast = c.view().toast.expect("toast");
    assert_eq!((toast.session_id, toast.text.as_str()), (3, "<b>done</b>"));
    assert_eq!(toast.expires_at, t0 + Duration::from_secs(4));

    c.update(ShellMsg::Tick, t0 + Duration::from_millis(3999));
    assert!(c.view().toast.is_some());
    c.update(ShellMsg::Tick, t0 + Duration::from_secs(4));
    assert!(c.view().toast.is_none());
}

#[test]
fn shell_ignores_foreign_bus_events() {
    let now = Instant::now();
    let mut c = ShellController::new(wide());
    let before = c.view();
    let fx = c.update(ShellMsg::Bus(BusEvent::LayoutChanged), now);
    assert!(fx.is_empty());
    assert_eq!(c.view(), before);
}
