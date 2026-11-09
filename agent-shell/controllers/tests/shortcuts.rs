//! S8-T-05: hotkeys: binding table, Esc, Ctrl+K alias, no focus trap.

use std::time::Instant;

use controllers::controllers::shortcut::{Msg, ShortcutController};
use controllers::keys::{Focus, Key, KeyPress, resolve};
use controllers::{
    BusEvent, Controller, Effect, EventBus, FocusColumn, SettingsPage, ShortcutAction,
};

fn press(key: Key, ctrl: bool, shift: bool, alt: bool, logo: bool) -> KeyPress {
    KeyPress {
        key,
        ctrl,
        shift,
        alt,
        logo,
    }
}

fn sup(c: char) -> KeyPress {
    press(Key::Char(c), false, false, false, true)
}

fn ctrl(c: char) -> KeyPress {
    press(Key::Char(c), true, false, false, false)
}

fn act(a: ShortcutAction) -> Option<BusEvent> {
    Some(BusEvent::ShortcutInvoked(a))
}

const ALL_FOCUS: [Focus; 3] = [Focus::Other, Focus::Terminal, Focus::Browser];

#[test]
fn super_bindings_work_in_every_focus() {
    let super_shift_n = press(Key::Char('n'), false, true, false, true);
    let table: Vec<(KeyPress, Option<BusEvent>)> = vec![
        (sup('k'), act(ShortcutAction::Search)),
        (sup('n'), act(ShortcutAction::NewSession)),
        (super_shift_n, act(ShortcutAction::NewFolder)),
        (sup('t'), act(ShortcutAction::NewThread)),
        (
            sup('1'),
            act(ShortcutAction::FocusColumn(FocusColumn::Notifications)),
        ),
        (
            sup('2'),
            act(ShortcutAction::FocusColumn(FocusColumn::Sessions)),
        ),
        (
            sup('3'),
            act(ShortcutAction::FocusColumn(FocusColumn::Chat)),
        ),
        (
            sup('4'),
            act(ShortcutAction::FocusColumn(FocusColumn::Tools)),
        ),
        (
            sup(','),
            Some(BusEvent::OpenSettings(SettingsPage::Account)),
        ),
    ];
    for (p, want) in table {
        for f in ALL_FOCUS {
            assert_eq!(resolve(p, f, 0), want, "{p:?} with focus {f:?}");
        }
    }
}

#[test]
fn super_shift_n_is_not_new_session_and_extra_modifiers_do_not_match() {
    assert_ne!(
        resolve(
            press(Key::Char('n'), false, true, false, true),
            Focus::Other,
            0
        ),
        act(ShortcutAction::NewSession)
    );
    assert_eq!(
        resolve(
            press(Key::Char('k'), false, false, true, true),
            Focus::Other,
            0
        ),
        None,
        "Super+Alt+K"
    );
    assert_eq!(
        resolve(
            press(Key::Char('1'), true, false, false, true),
            Focus::Other,
            0
        ),
        None,
        "Super+Ctrl+1"
    );
    assert_eq!(
        resolve(sup('5'), Focus::Other, 0),
        None,
        "there are four columns"
    );
    assert_eq!(resolve(sup('0'), Focus::Other, 0), None);
}

#[test]
fn ctrl_k_is_search_alias_only_outside_terminal_and_browser() {
    assert_eq!(
        resolve(ctrl('k'), Focus::Other, 0),
        act(ShortcutAction::Search)
    );
    assert_eq!(
        resolve(ctrl('k'), Focus::Terminal, 0),
        None,
        "Ctrl+K is read by readline"
    );
    assert_eq!(
        resolve(ctrl('k'), Focus::Browser, 0),
        None,
        "Ctrl+K is read by the page"
    );
}

#[test]
fn terminal_keys_are_not_stolen() {
    for c in ['n', 't', 'a', 'e', 'u', 'w', '1', ','] {
        for f in ALL_FOCUS {
            assert_eq!(
                resolve(ctrl(c), f, 0),
                None,
                "Ctrl+{c} with focus {f:?} is not a shell hotkey"
            );
        }
    }
    for c in ['k', 'n', 't'] {
        assert_eq!(
            resolve(
                press(Key::Char(c), false, false, false, false),
                Focus::Terminal,
                0
            ),
            None
        );
    }
}

#[test]
fn escape_closes_top_layer_only_when_one_is_open() {
    let esc = press(Key::Escape, false, false, false, false);
    assert_eq!(
        resolve(esc, Focus::Other, 1),
        act(ShortcutAction::CloseTopLayer)
    );
    assert_eq!(
        resolve(esc, Focus::Other, 3),
        act(ShortcutAction::CloseTopLayer)
    );
    assert_eq!(resolve(esc, Focus::Other, 0), None, "nothing to close");
    assert_eq!(
        resolve(esc, Focus::Terminal, 0),
        None,
        "Esc goes to the terminal (vim, readline)"
    );
    assert_eq!(
        resolve(esc, Focus::Terminal, 1),
        act(ShortcutAction::CloseTopLayer),
        "a dialog over the terminal closes"
    );
}

#[test]
fn controller_publishes_to_bus_and_tracks_layers() {
    let bus = EventBus::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s = seen.clone();
    bus.subscribe(move |e| s.borrow_mut().push(e.clone()));
    let mut c = ShortcutController::default();
    let esc = press(Key::Escape, false, false, false, false);
    let send = |c: &mut ShortcutController, m: Msg| {
        for e in c.update(m, Instant::now()) {
            if let Effect::Publish(ev) = e {
                bus.publish(ev);
            }
        }
    };
    send(
        &mut c,
        Msg::Key {
            press: esc,
            focus: Focus::Other,
        },
    );
    assert!(seen.borrow().is_empty());
    send(&mut c, Msg::Layers(2));
    assert_eq!(c.view().layers, 2);
    send(
        &mut c,
        Msg::Key {
            press: esc,
            focus: Focus::Other,
        },
    );
    send(
        &mut c,
        Msg::Key {
            press: sup('3'),
            focus: Focus::Terminal,
        },
    );
    assert_eq!(
        seen.borrow().clone(),
        vec![
            BusEvent::ShortcutInvoked(ShortcutAction::CloseTopLayer),
            BusEvent::ShortcutInvoked(ShortcutAction::FocusColumn(FocusColumn::Chat)),
        ]
    );
}

#[test]
fn consumes_matches_resolve_so_terminal_gets_its_keys() {
    let c = ShortcutController::default();
    assert!(
        c.consumes(sup('1'), Focus::Terminal),
        "Super+1 leaves the terminal"
    );
    assert!(c.consumes(sup('1'), Focus::Browser));
    assert!(c.consumes(ctrl('k'), Focus::Other));
    assert!(
        !c.consumes(ctrl('k'), Focus::Terminal),
        "Ctrl+K goes to the terminal"
    );
    assert!(
        !c.consumes(ctrl('n'), Focus::Terminal),
        "Ctrl+N goes to the terminal"
    );
    assert!(!c.consumes(ctrl('n'), Focus::Other));
}
