//! `agent-shell` window (GTK4 + libadwaita): the bar and four columns, themes.
//!
//! `--smoke`: show the window, wait for the first frame and exit with code 0 (within 10 seconds, otherwise code 1).

#[cfg(target_os = "linux")]
mod app;

#[cfg(target_os = "linux")]
fn main() -> gtk4::glib::ExitCode {
    app::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("agent-shell builds and runs on Linux only (GTK4 + libadwaita)");
    std::process::exit(2);
}
