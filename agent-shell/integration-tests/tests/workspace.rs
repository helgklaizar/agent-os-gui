//! Workspace layout checks via the real `cargo metadata` and `git`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels up")
        .to_path_buf()
}

fn metadata() -> serde_json::Value {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root())
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed");
    serde_json::from_slice(&out.stdout).expect("cargo metadata prints JSON")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_default()
}

#[test]
fn workspace_members_match_sprint() {
    let meta = metadata();
    let mut names: Vec<String> = meta["packages"]
        .as_array()
        .expect("packages is an array")
        .iter()
        .map(|p| p["name"].as_str().expect("name").to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            // browser client
            "agent-web",
            // agentd/* members
            "agentd",
            "agentd-api",
            "agentd-auth",
            "agentd-core",
            "agentd-db",
            "agentd-e2e",
            "agentd-providers",
            "agentd-sys",
            "controllers",
            "core-client",
            "fake-agentd",
            "integration-tests",
            "ui"
        ]
    );
}

#[test]
fn rust_version_equals_toolchain_channel() {
    let toolchain = read("rust-toolchain.toml");
    let channel = toolchain
        .lines()
        .find_map(|l| l.strip_prefix("channel = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("channel in rust-toolchain.toml");
    let meta = metadata();
    for p in meta["packages"].as_array().expect("packages") {
        assert_eq!(
            p["rust_version"].as_str(),
            Some(channel.as_str()),
            "rust-version of package {} does not equal the toolchain",
            p["name"]
        );
        assert_eq!(p["edition"].as_str(), Some("2024"));
    }
}

#[test]
fn target_and_fake_data_are_ignored() {
    for path in ["target/x", "fake-agentd/.data/x"] {
        let ok = Command::new("git")
            .args(["check-ignore", "-q", path])
            .current_dir(root())
            .status()
            .expect("git runs")
            .success();
        assert!(ok, "{path} is not ignored by git");
    }
}

#[test]
fn cargo_lock_is_tracked() {
    let out = Command::new("git")
        .args(["ls-files", "--error-unmatch", "Cargo.lock"])
        .current_dir(root())
        .output()
        .expect("git runs");
    assert!(out.status.success(), "Cargo.lock is not tracked by git");
}

#[test]
fn unwrap_is_denied_in_core_client_and_controllers() {
    for lib in [
        "agent-shell/core-client/src/lib.rs",
        "agent-shell/controllers/src/lib.rs",
    ] {
        assert!(
            read(lib).contains("#![deny(clippy::unwrap_used)]"),
            "{lib}: no deny(clippy::unwrap_used)"
        );
    }
}

#[test]
fn unsafe_is_forbidden_outside_ui() {
    let root_toml = read("Cargo.toml");
    assert!(root_toml.contains("[workspace.lints.rust]"));
    assert!(root_toml.contains("unsafe_code = \"forbid\""));
    for m in [
        "agent-shell/core-client",
        "agent-shell/controllers",
        "fake-agentd",
        "agent-shell/integration-tests",
    ] {
        let toml = read(&format!("{m}/Cargo.toml"));
        let compact: String = toml.split_whitespace().collect();
        assert!(
            compact.contains("[lints]workspace=true"),
            "{m}: no lints inheritance"
        );
    }
}
