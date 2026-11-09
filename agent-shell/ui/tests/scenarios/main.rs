//! UI scenarios under headless Wayland (`weston --backend=headless`): `cargo test -p ui --test scenarios`.
//!
//! Own runner (`harness = false`): GTK is initialised in one thread and scenarios run in turn. Each scenario
//! prints `test <name> ... ok|FAILED`; the exit code is non-zero if at least one failed. The argument is a name filter.
//! No `weston`, `dbus-run-session` or `python3` with `gi`: the scenario fails with a clear reason instead of being skipped.

#[cfg(target_os = "linux")]
mod a11y;
#[cfg(target_os = "linux")]
mod bar;
#[cfg(target_os = "linux")]
mod bar_a11y;
#[cfg(target_os = "linux")]
mod bar_contrast;
#[cfg(target_os = "linux")]
mod bar_find;
#[cfg(target_os = "linux")]
mod bar_flow;
#[cfg(target_os = "linux")]
mod chat;
#[cfg(target_os = "linux")]
mod chat_a11y;
#[cfg(target_os = "linux")]
mod chat_perf;
#[cfg(target_os = "linux")]
mod ctx;
#[cfg(target_os = "linux")]
mod layout;
#[cfg(target_os = "linux")]
mod notif;
#[cfg(target_os = "linux")]
mod notif_a11y;
#[cfg(target_os = "linux")]
mod settings;
#[cfg(target_os = "linux")]
mod settings_a11y;
#[cfg(target_os = "linux")]
mod settings_more;
#[cfg(target_os = "linux")]
mod smoke;
#[cfg(target_os = "linux")]
mod theme;
#[cfg(target_os = "linux")]
mod tools;
#[cfg(target_os = "linux")]
mod tools_a11y;
#[cfg(target_os = "linux")]
mod tools_settings;
#[cfg(target_os = "linux")]
mod tools_term;
#[cfg(target_os = "linux")]
mod tree;
#[cfg(target_os = "linux")]
mod tree_a11y;
#[cfg(target_os = "linux")]
mod tree_bulk;
#[cfg(target_os = "linux")]
mod tree_dialogs;
#[cfg(target_os = "linux")]
mod tree_selected;
#[cfg(target_os = "linux")]
mod weston;

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("scenarios: UI scenarios run only on Linux (dev-vm); nothing is run on this OS");
}

#[cfg(target_os = "linux")]
fn main() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let filter = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    let ctx = ctx::Ctx::new();
    let scenarios: Vec<(&str, ctx::Scenario)> = [
        a11y::scenarios(),
        bar::scenarios(),
        bar_flow::scenarios(),
        chat::scenarios(),
        chat_a11y::scenarios(),
        chat_perf::scenarios(),
        layout::scenarios(),
        notif::scenarios(),
        notif_a11y::scenarios(),
        settings::scenarios(),
        settings_a11y::scenarios(),
        settings_more::scenarios(),
        smoke::scenarios(),
        theme::scenarios(),
        tools::scenarios(),
        tools_a11y::scenarios(),
        tools_settings::scenarios(),
        tools_term::scenarios(),
        tree::scenarios(),
        tree_a11y::scenarios(),
        tree_bulk::scenarios(),
        tree_dialogs::scenarios(),
        tree_selected::scenarios(),
    ]
    .concat();

    let (mut failed, mut ran) = (0, 0);
    for (name, run) in scenarios {
        if filter
            .as_deref()
            .is_some_and(|f| !f.split(',').any(|part| name.contains(part)))
        {
            continue;
        }
        ran += 1;
        let result = catch_unwind(AssertUnwindSafe(|| run(&ctx)))
            .unwrap_or_else(|_| Err("scenario panicked".to_string()));
        ctx.close_windows();
        match result {
            Ok(()) => println!("test {name} ... ok"),
            Err(why) => {
                failed += 1;
                println!(
                    "test {name} ... FAILED\n    {}",
                    why.replace('\n', "\n    ")
                );
            }
        }
    }
    println!("\nscenarios: ran {ran}, failed {failed}");
    std::process::exit(i32::from(failed > 0 || ran == 0));
}
