//! Bar indicator model: manifest, state, catalogue, collapsing.

mod catalog;
mod collapse;
mod model;
mod quiet;

pub use catalog::{DEFAULT_ENABLED, catalog, manifest};
pub use collapse::{BarItem, Collapsed, capacity_for_width, collapse};
pub use model::{ClickAction, Group, Manifest, Rule, State, agentd_state, state_for};
pub use quiet::QuietHours;
