//! Keyboard shortcuts: the binding table. Global ones use `Super` and work with any focus, including the terminal
//! and browser (an emergency exit from a focus trap). `Ctrl+K` is a search alias only outside the terminal and browser: there it is read by
//! readline and the page. The controller publishes an event to the bus; the owner performs the action.

use crate::bus::BusEvent;
use crate::types::{FocusColumn, SettingsPage, ShortcutAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Lowercase character (`Shift` is passed as a flag).
    Char(char),
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The `Super` key.
    pub logo: bool,
}

/// Where the focus is at the moment of the key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Other,
    Terminal,
    Browser,
}

fn invoked(a: ShortcutAction) -> Option<BusEvent> {
    Some(BusEvent::ShortcutInvoked(a))
}

/// What the key press does. `layers` is the number of open dialogs and palettes; `Esc` closes the topmost only if there is one,
/// otherwise it goes to the terminal and browser (vim, readline).
pub fn resolve(press: KeyPress, focus: Focus, layers: usize) -> Option<BusEvent> {
    let KeyPress {
        key,
        ctrl,
        shift,
        alt,
        logo,
    } = press;
    if alt {
        return None;
    }
    match (key, logo, ctrl, shift) {
        (Key::Escape, false, false, false) if layers > 0 => invoked(ShortcutAction::CloseTopLayer),
        (Key::Char('k'), true, false, false) => invoked(ShortcutAction::Search),
        (Key::Char('k'), false, true, false) if focus == Focus::Other => {
            invoked(ShortcutAction::Search)
        }
        (Key::Char('n'), true, false, false) => invoked(ShortcutAction::NewSession),
        (Key::Char('n'), true, false, true) => invoked(ShortcutAction::NewFolder),
        (Key::Char('t'), true, false, false) => invoked(ShortcutAction::NewThread),
        (Key::Char(c @ '1'..='4'), true, false, false) => {
            invoked(ShortcutAction::FocusColumn(match c {
                '1' => FocusColumn::Notifications,
                '2' => FocusColumn::Sessions,
                '3' => FocusColumn::Chat,
                _ => FocusColumn::Tools,
            }))
        }
        (Key::Char(','), true, false, false) => Some(BusEvent::OpenSettings(SettingsPage::Account)),
        _ => None,
    }
}
