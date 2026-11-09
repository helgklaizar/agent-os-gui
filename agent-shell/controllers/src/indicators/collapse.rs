//! Visibility and priority collapsing of bar indicators: red ones are always visible, then yellow, then the rest
//! in the order from Settings.

use super::State;

/// Width of one indicator in the bar, px (assumption: an icon and up to 5 value characters at the bar font).
pub const ITEM_WIDTH_PX: u32 = 96;
/// Room for the other bar parts (clock, column toggles, Settings), px (assumption).
/// Calculation: a 1024 px window (the minimum) fits the default set of 8 indicators: (1024 − 256) / 96 = 8.
pub const RESERVED_PX: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarItem {
    pub id: String,
    pub state: State,
    pub position: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Collapsed {
    /// Visible, in Settings order.
    pub visible: Vec<String>,
    /// Hidden, in Settings order.
    pub hidden: Vec<String>,
}

/// How many indicators fit in a bar of the given width.
pub fn capacity_for_width(width_px: u32) -> usize {
    (width_px.saturating_sub(RESERVED_PX) / ITEM_WIDTH_PX) as usize
}

fn rank(s: State) -> u8 {
    match s {
        State::Bad => 0,
        State::Warn => 1,
        _ => 2,
    }
}

/// Picks visible indicators: all red ones, the remaining room by priority. Disabled ones are ignored.
pub fn collapse(items: &[BarItem], capacity: usize) -> Collapsed {
    let mut enabled: Vec<&BarItem> = items.iter().filter(|i| i.enabled).collect();
    enabled.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
    let bad = enabled.iter().filter(|i| i.state == State::Bad).count();
    let shown = capacity.max(bad);
    let mut by_priority = enabled.clone();
    by_priority.sort_by_key(|i| rank(i.state)); // stable: within a rank, Settings order is kept
    let keep: Vec<&str> = by_priority
        .iter()
        .take(shown)
        .map(|i| i.id.as_str())
        .collect();
    let mut out = Collapsed::default();
    for i in enabled {
        if keep.contains(&i.id.as_str()) {
            out.visible.push(i.id.clone());
        } else {
            out.hidden.push(i.id.clone());
        }
    }
    out
}
