//! Path to the `agentd` binary for end-to-end tests: rebuilt once per process.
//!
//! `cargo test` does not rebuild the core binary: a stale `agentd` gives false red and false green probes.
//! Tests must not build their own `target/<profile>/agentd` path; only this function does (guarded by `bin_fresh.rs`).

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn agentd_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>");
    let bin = dir.join("agentd");
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args([
            "build",
            "--locked",
            "-p",
            "agentd",
            "--features",
            "test-support",
        ])
        .arg("--target-dir")
        .arg(dir.parent().expect("target"));
        if dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo build -p agentd");
        assert!(status.success(), "cargo build -p agentd failed: {status}");
    });
    assert!(
        bin.exists(),
        "missing {}: build with `cargo build -p agentd`",
        bin.display()
    );
    bin
}
