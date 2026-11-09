//! Headless compositor for scenarios: its own `XDG_RUNTIME_DIR` and socket, so as not to touch a foreign session.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Weston {
    child: Child,
    runtime: tempfile::TempDir,
    socket: String,
}

impl Weston {
    /// Starts `weston --backend=headless` of size `w`×`h` and waits up to 10 seconds for the socket.
    pub fn start(w: u32, h: u32) -> Weston {
        let runtime = tempfile::Builder::new()
            .prefix("agent-shell-ui-")
            .tempdir()
            .expect("XDG_RUNTIME_DIR directory for weston");
        set_mode_0700(runtime.path());
        let socket = format!("agent-test-{}", std::process::id());
        let log = File::create(runtime.path().join("weston.log")).expect("weston log");
        let child = Command::new("weston")
            .args(["--backend=headless", "--renderer=pixman", "--no-config"])
            .arg(format!("--width={w}"))
            .arg(format!("--height={h}"))
            .arg(format!("--socket={socket}"))
            .env("XDG_RUNTIME_DIR", runtime.path())
            .stdout(Stdio::from(log.try_clone().expect("log file duplicate")))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("weston did not start (the weston package is needed on dev-vm)");
        let weston = Weston {
            child,
            runtime,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !weston.runtime.path().join(&weston.socket).exists() {
            assert!(
                Instant::now() < deadline,
                "weston did not create the socket within 10 seconds, log: {}",
                weston.runtime.path().join("weston.log").display()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        weston
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.runtime.path().to_path_buf()
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// Wayland client environment: a separate directory, socket, without portals and without X11.
    pub fn apply_env(&self, cmd: &mut Command) {
        cmd.env("XDG_RUNTIME_DIR", self.runtime.path())
            .env("WAYLAND_DISPLAY", &self.socket)
            .env("GDK_BACKEND", "wayland")
            .env("GSK_RENDERER", "cairo");
    }
}

impl Drop for Weston {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn set_mode_0700(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("0700 permissions on XDG_RUNTIME_DIR");
}
