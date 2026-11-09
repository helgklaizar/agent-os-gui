//! GTK4 + libadwaita views and everything they need.
//!
//! GTK exists only on Linux: on macOS the crate builds as an empty skeleton, so everything that depends on GTK is under
//! `cfg(target_os = "linux")`. The rest (colour tokens, theme exchange with the core) is tested on both OSes.

#[cfg(target_os = "linux")]
pub mod a11y;
pub mod bar_feed;
#[cfg(target_os = "linux")]
pub mod bar_rig;
#[cfg(target_os = "linux")]
pub mod bar_view;
#[cfg(target_os = "linux")]
mod bar_worker;
#[cfg(target_os = "linux")]
mod browser_agent;
#[cfg(target_os = "linux")]
pub mod browser_pane;
#[cfg(target_os = "linux")]
mod browser_start;
#[cfg(target_os = "linux")]
mod chat_apply;
pub mod chat_exec;
pub mod chat_feed;
#[cfg(target_os = "linux")]
mod chat_items;
#[cfg(target_os = "linux")]
mod chat_md;
#[cfg(target_os = "linux")]
mod chat_rig;
#[cfg(target_os = "linux")]
pub mod chat_widgets;
#[cfg(target_os = "linux")]
mod chat_worker;
pub mod conn_feed;
#[cfg(target_os = "linux")]
pub mod core_worker;
#[cfg(target_os = "linux")]
mod dialog_form_view;
#[cfg(target_os = "linux")]
mod dialog_host;
#[cfg(target_os = "linux")]
pub mod hardening;
pub mod modules_feed;
pub mod notif_exec;
pub mod notif_feed;
#[cfg(target_os = "linux")]
mod notif_rig;
#[cfg(target_os = "linux")]
pub mod notif_toast;
#[cfg(target_os = "linux")]
pub mod notif_widgets;
#[cfg(target_os = "linux")]
mod notif_worker;
pub mod prefs_io;
pub mod settings_exec;
pub mod settings_feed;
#[cfg(target_os = "linux")]
mod settings_field_view;
#[cfg(target_os = "linux")]
pub mod settings_rig;
#[cfg(target_os = "linux")]
pub mod settings_rows;
#[cfg(target_os = "linux")]
pub mod settings_widgets;
#[cfg(target_os = "linux")]
mod settings_worker;
#[cfg(target_os = "linux")]
pub mod shell;
#[cfg(target_os = "linux")]
mod shell_access;
#[cfg(target_os = "linux")]
mod shell_inner;
#[cfg(target_os = "linux")]
mod shell_size;
#[cfg(target_os = "linux")]
pub mod shell_view;
#[cfg(target_os = "linux")]
pub mod ss_pane;
#[cfg(target_os = "linux")]
mod ss_pane_wire;
#[cfg(target_os = "linux")]
mod ss_perms;
#[cfg(target_os = "linux")]
mod ss_tasks;
#[cfg(target_os = "linux")]
pub mod terminal_pane;
pub mod theme_sync;
pub mod theme_tokens;
pub mod tools_exec;
pub mod tools_feed;
#[cfg(target_os = "linux")]
pub mod tools_rig;
#[cfg(target_os = "linux")]
pub mod tools_widgets;
#[cfg(target_os = "linux")]
mod tools_worker;
pub mod tree_exec;
pub mod tree_feed;
#[cfg(target_os = "linux")]
mod tree_rig;
#[cfg(target_os = "linux")]
mod tree_row;
#[cfg(target_os = "linux")]
mod tree_row_input;
#[cfg(target_os = "linux")]
pub mod tree_widgets;
#[cfg(target_os = "linux")]
mod tree_worker;
