//! `ui_smoke_window`: a real `agent-shell --smoke` process draws its first frame and exits with code 0 within 10 seconds.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::ctx::{Ctx, Scenario};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_smoke_window", smoke_ok),
        (
            "ui_smoke_fails_without_display",
            smoke_fails_without_display,
        ),
        ("ui_process_not_dumpable", not_dumpable),
    ]
}

fn wait_exit(mut child: Child, limit: Duration) -> Result<(Option<i32>, Duration), String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok((status.code(), started.elapsed()));
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("the process did not finish within {limit:?}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The process is isolated from the user bus: on the VM the environment holds `DBUS_SESSION_BUS_ADDRESS` of a live session, and
/// `GtkApplication` synchronously asks it for `xdg-desktop-portal`, which without the user's display sits in `activating`;
/// the window never reaches its first frame, and the exit on the `--smoke` timer does not fire either (the main thread is busy with the call).
/// A bus that does not exist refuses at once, and the application works without it.
fn agent_shell(ctx: &Ctx) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agent-shell"));
    ctx.weston.apply_env(&mut cmd);
    cmd.env_remove("DISPLAY")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!(
                "unix:path={}/no-session-bus",
                ctx.weston.runtime_dir().display()
            ),
        )
        .stdin(Stdio::null());
    cmd
}

fn smoke_ok(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let child = agent_shell(ctx)
        .arg("--smoke")
        .env("XDG_CONFIG_HOME", dir.path())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (code, took) = wait_exit(child, Duration::from_secs(10))?;
    if code == Some(0) {
        println!("    --smoke finished in {took:.1?}");
        Ok(())
    } else {
        Err(format!("--smoke finished with code {code:?} in {took:?}"))
    }
}

fn smoke_fails_without_display(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let child = agent_shell(ctx)
        .arg("--smoke")
        .env("XDG_CONFIG_HOME", dir.path())
        .env("WAYLAND_DISPLAY", "no-such-wayland-socket")
        .spawn()
        .map_err(|e| e.to_string())?;
    let (code, _) = wait_exit(child, Duration::from_secs(10))?;
    match code {
        Some(0) | None => Err(format!(
            "without a compositor a non-zero code was expected, got {code:?}"
        )),
        Some(_) => Ok(()),
    }
}

fn not_dumpable(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut child = agent_shell(ctx)
        .env("XDG_CONFIG_HOME", dir.path())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();
    std::thread::sleep(Duration::from_secs(3));
    let owner = fs::metadata(format!("/proc/{pid}/environ")).map(|m| m.uid());
    let readable = fs::read(format!("/proc/{pid}/environ")).is_ok();
    let _ = child.kill();
    let _ = child.wait();
    match owner {
        Ok(0) if !readable => Ok(()),
        other => Err(format!(
            "process is dumpable: owner of /proc/{pid}/environ {other:?}, readable by a neighbour: {readable}"
        )),
    }
}
