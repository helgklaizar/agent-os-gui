//! A7-12: contents of the core `.deb` package: `control` fields, the service unit, maintainer scripts (they do not start the service, do not touch
//! the DB or user data), `ptrace_scope`, the build script. Build and install are checked on dev-vm.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn pkg() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../packaging")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(pkg().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn packaging_control_has_required_fields() {
    let c = read("deb/DEBIAN/control.in");
    for f in [
        "Package: agentd",
        "Version: @VERSION@",
        "Architecture: @ARCH@",
        "Depends:",
        "Recommends:",
        "Description:",
    ] {
        assert!(c.contains(f), "no {f}");
    }
    assert!(
        !c.to_lowercase().contains("agent-shell"),
        "core does not depend on the GUI"
    );
}

#[test]
fn packaging_user_unit_is_hardened_and_secret_free() {
    let u = read("deb/usr/lib/systemd/user/agentd.service");
    assert!(u.contains("ExecStart=/usr/bin/agentd serve"));
    for must in [
        "NoNewPrivileges=true",
        "ProtectSystem=strict",
        "UMask=0077",
        "WantedBy=default.target",
    ] {
        assert!(u.contains(must), "no {must}");
    }
    for bad in ["TOKEN", "KEY=", "PASSWORD", "SECRET", "Environment="] {
        assert!(
            !u.to_uppercase().contains(&bad.to_uppercase()),
            "secret or environment in the unit: {bad}"
        );
    }
}

#[test]
fn packaging_maintainer_scripts_never_start_services_or_touch_data() {
    for s in ["postinst", "prerm", "postrm"] {
        let t = read(&format!("deb/DEBIAN/{s}"));
        assert!(t.starts_with("#!/bin/sh\n"), "{s}");
        let code: String = t
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        for bad in [
            "systemctl",
            "rm ",
            "rm\t",
            "unlink",
            "sqlite",
            "sessions.db",
            "keys.env",
            "--now",
        ] {
            assert!(!code.contains(bad), "{s}: \"{bad}\" in script code");
        }
        let mode = std::fs::metadata(pkg().join(format!("deb/DEBIAN/{s}")))
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "{s} is not executable");
    }
}

#[test]
fn packaging_ptrace_scope_and_runner_user() {
    assert!(read("deb/usr/lib/sysctl.d/90-agentd.conf").contains("kernel.yama.ptrace_scope = 1"));
    let u = read("deb/usr/lib/sysusers.d/agentd.conf");
    assert!(u.contains("agentos-run") && u.contains("/usr/sbin/nologin"));
}

#[test]
fn packaging_build_script_is_valid_and_names_versions() {
    let ok = Command::new("bash")
        .arg("-n")
        .arg(pkg().join("build-deb.sh"))
        .status()
        .unwrap();
    assert!(ok.success(), "syntax of build-deb.sh");
    let s = read("build-deb.sh");
    assert!(
        s.contains("chmod -R go-w"),
        "package file permissions without group and other write"
    );
    assert!(s.contains("~ubuntu") && s.contains("--locked") && s.contains("--root-owner-group"));
}

#[test]
fn packaging_runner_units_and_scripts_are_hardened() {
    let sock = read("deb/usr/lib/systemd/system/agent-runner.socket");
    assert!(sock.contains("Accept=yes") && sock.contains("SocketMode=0600"));
    assert!(sock.contains("/run/agent-runner.sock"));
    let svc = read("deb/usr/lib/systemd/system/agent-runner@.service");
    for need in [
        "StandardInput=socket",
        "User=agentos-run",
        "NoNewPrivileges=true",
        "ProtectSystem=strict",
        "CapabilityBoundingSet=",
        "ExecStart=/usr/bin/agentd runner",
    ] {
        assert!(svc.contains(need), "{need}");
    }
    for s in ["agent-runner-install.sh", "agent-runner-verify.sh"] {
        let ok = Command::new("bash")
            .arg("-n")
            .arg(pkg().join(s))
            .status()
            .unwrap();
        assert!(ok.success(), "syntax of {s}");
        let text = read(s);
        assert!(!text.contains("rm -"), "{s}: no deletion");
    }
    assert!(read("agent-runner-install.sh").contains("backup"));
}

#[test]
fn lintian_errors_are_closed_in_sources() {
    // A-core-100: lintian on a rebuilt package gave bogus-mail-host, no-copyright-file, no-changelog and long lines.
    let c = read("deb/DEBIAN/control.in");
    let maintainer = c.lines().find(|l| l.starts_with("Maintainer:")).unwrap();
    assert!(!maintainer.contains("localhost"), "{maintainer}");
    for l in c.lines().filter(|l| l.starts_with(' ')) {
        assert!(
            l.chars().count() <= 80,
            "description line longer than 80: {l}"
        );
    }
    let b = read("build-deb.sh");
    for need in ["usr/share/doc/agentd/copyright", "changelog.Debian.gz"] {
        assert!(b.contains(need), "build does not install {need}");
    }
    assert!(
        read("deb/usr/share/doc/agentd/copyright").contains("Copyright"),
        "copyright file in the package tree"
    );
}

#[test]
fn runner_resource_limits_are_set_in_the_unit() {
    // A7-01: a fork bomb and a memory hog are limited by the runner unit (`runner_resource_limits`).
    let svc = read("deb/usr/lib/systemd/system/agent-runner@.service");
    for need in ["MemoryMax=", "TasksMax=", "CPUQuota="] {
        assert!(svc.contains(need), "runner unit lacks {need}");
    }
    for bad in ["MemoryMax=infinity", "TasksMax=infinity"] {
        assert!(!svc.contains(bad), "{bad}");
    }
}

#[test]
fn runner_no_network_unit_closes_inet() {
    // A7-01 ("no network"): agent commands run in the runner unit; the network is closed by a namespace and address families.
    let svc = read("deb/usr/lib/systemd/system/agent-runner@.service");
    let lines: Vec<&str> = svc.lines().map(str::trim).collect();
    assert!(
        lines.contains(&"PrivateNetwork=true"),
        "runner unit lacks PrivateNetwork=true"
    );
    assert!(
        lines.contains(&"RestrictAddressFamilies=AF_UNIX"),
        "runner address families: AF_UNIX only"
    );
    for bad in ["AF_INET", "AF_INET6", "AF_PACKET", "AF_NETLINK"] {
        assert!(
            !lines
                .iter()
                .filter(|l| !l.starts_with('#'))
                .any(|l| l.contains(bad)),
            "{bad} in the runner unit"
        );
    }
    assert!(
        lines.contains(&"InaccessiblePaths=/run/user"),
        "/run/user (core socket, tokens) is inaccessible to the agent command"
    );
}

#[test]
fn core_unit_does_not_restart_on_db_newer_than_code() {
    // Exit code 3 ("DB newer than code") is not cured by restart: a restart loop would only torment the DB.
    let svc = read("deb/usr/lib/systemd/user/agentd.service");
    assert!(
        svc.lines()
            .any(|l| l.trim() == "RestartPreventExitStatus=3")
    );
}

#[test]
fn packaging_installs_web_client_when_built() {
    let b = read("build-deb.sh");
    assert!(
        b.contains("/usr/share/agentd/web"),
        "client is installed to the core's default directory"
    );
    assert!(b.contains("agent-web/dist"));
    assert!(b.contains("WITH_WEB"), "enabled and disabled by a variable");
    // The default directory in the core matches the package directory.
    let cli = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs"))
        .unwrap();
    assert!(cli.contains("\"/usr/share/agentd/web\""));
}
