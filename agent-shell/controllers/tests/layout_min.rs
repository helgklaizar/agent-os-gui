//! Minimum window size: 1024×640, the chat is never narrower than 480 px under any automatic and manual
//! "collapsed" states; manual expansion may narrow the chat, but the width arithmetic always adds up.
//! The window-size UI scenario on real GTK is `ui_window_min_size_1024x640` (Linux).

use controllers::layout::{CHAT_MIN, MIN_WINDOW, resolve};
use controllers::prefs::ColumnsPrefs;
use controllers::types::Manual;

fn prefs(n: Manual, s: Manual, t: Manual) -> ColumnsPrefs {
    let mut p = ColumnsPrefs::default();
    p.notifications.manual = n;
    p.sessions.manual = s;
    p.tools.manual = t;
    p
}

const STATES: [Manual; 3] = [Manual::Auto, Manual::Collapsed, Manual::Expanded];

#[test]
fn min_window_is_1024_by_640() {
    assert_eq!(MIN_WINDOW, (1024, 640));
    assert_eq!(CHAT_MIN, 480);
}

#[test]
fn chat_is_never_narrower_than_480_without_manual_expansion() {
    // arithmetic at the minimum width: 56 + 236 + 56 + chat = 1024 → chat 676
    for w in MIN_WINDOW.0..=3000 {
        for n in [Manual::Auto, Manual::Collapsed] {
            for s in [Manual::Auto, Manual::Collapsed] {
                for t in [Manual::Auto, Manual::Collapsed] {
                    let l = resolve(w, &prefs(n, s, t));
                    assert!(
                        l.chat_w >= CHAT_MIN,
                        "w={w} {n:?}/{s:?}/{t:?}: chat {} px",
                        l.chat_w
                    );
                }
            }
        }
    }
}

#[test]
fn widths_always_add_up_to_the_window() {
    for w in (MIN_WINDOW.0..=2400).step_by(7) {
        for n in STATES {
            for s in STATES {
                for t in STATES {
                    let l = resolve(w, &prefs(n, s, t));
                    let sum = l.notifications_w + l.sessions_w + l.tools_w + l.chat_w;
                    // the chat is never negative: if the columns are wider than the window it is zero, and the manual choice is not overridden
                    assert!(
                        sum == w || l.chat_w == 0,
                        "w={w} {n:?}/{s:?}/{t:?}: sum {sum}"
                    );
                }
            }
        }
    }
}

#[test]
fn manual_expansion_may_narrow_chat_but_auto_columns_give_way_first() {
    // notifications manually expanded at 1024: tools (auto) yield, chat 1024 − 300 − 236 − 56 = 432
    let l = resolve(1024, &prefs(Manual::Expanded, Manual::Auto, Manual::Auto));
    assert!(!l.notifications_collapsed && l.tools_collapsed);
    assert_eq!(l.chat_w, 432);
    // the manual "expanded" choice is not overridden, even if the chat went below the minimum
    assert!(l.chat_w < CHAT_MIN);
    // all three manually expanded: the chat is squeezed to zero, the columns stay in place
    let l = resolve(
        1024,
        &prefs(Manual::Expanded, Manual::Expanded, Manual::Expanded),
    );
    assert_eq!(
        (l.notifications_w, l.sessions_w, l.tools_w),
        (300, 236, 560)
    );
    assert_eq!(l.chat_w, 0);
}

#[test]
fn boundary_widths_around_min_window() {
    // 1023 is below the minimum, a window is never that small, but the function does not panic
    let l = resolve(
        MIN_WINDOW.0 - 1,
        &prefs(Manual::Auto, Manual::Auto, Manual::Auto),
    );
    assert_eq!(l.chat_w, 1023 - 56 - 236 - 56);
    let l = resolve(
        MIN_WINDOW.0,
        &prefs(Manual::Auto, Manual::Auto, Manual::Auto),
    );
    assert_eq!(l.chat_w, 676);
    let l = resolve(
        MIN_WINDOW.0 + 1,
        &prefs(Manual::Auto, Manual::Auto, Manual::Auto),
    );
    assert_eq!(l.chat_w, 677);
}
