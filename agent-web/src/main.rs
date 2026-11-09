//! Browser client of Agent OS. All logic lives in `controllers`; this crate holds only transport and view.

mod api;
mod app;
mod attach;
mod core;
mod design;
mod icons;
mod indicators;
mod md_view;
mod notes;
mod outbox;
mod settings;
mod term;
mod tools;
mod util;
mod view_bar;
mod view_chat;
mod view_notes;
mod view_rail;
mod view_settings;
mod view_tools;
mod view_tree;

fn main() {
    leptos::mount::mount_to_body(app::App);
}
