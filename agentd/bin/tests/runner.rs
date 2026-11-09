//! A7-01: `agentd runner` (agent runner under socket-activation `Accept=yes`: the accepted connection is standard input). Protocol:
//! one JSON request line `{cwd, command, timeout_secs?, max_output?}`, in reply `{stream, seq, data}` lines and the result
//! `{exit, duration_ms, truncated, timeout}` or a refusal `{denied}`. Checks: peer uid, directories, extra fields, environment.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Rig {
    dir: tempfile::TempDir,
    project: PathBuf,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().canonicalize().unwrap().join("project");
    std::fs::create_dir_all(&project).unwrap();
    Rig { dir, project }
}

/// Starts the runner with the accepted connection on standard input and returns the response frames.
fn ask(allow_uid: u32, dirs: &[&Path], req: &Value) -> Vec<Value> {
    ask_env(allow_uid, dirs, req, &[])
}

/// Same, but the runner process gets extra environment variables (probe for leakage into the command's environment).
fn ask_env(allow_uid: u32, dirs: &[&Path], req: &Value, env: &[(&str, &str)]) -> Vec<Value> {
    let (mine, theirs) = UnixStream::pair().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.arg("runner")
        .args(["--allow-uid", &allow_uid.to_string()])
        .envs(env.iter().copied());
    for d in dirs {
        c.arg("--allow-dir").arg(d);
    }
    let mut child = c
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = mine.try_clone().unwrap();
    writeln!(w, "{req}").unwrap();
    let started = Instant::now();
    mine.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut frames: Vec<Value> = Vec::new();
    for l in BufReader::new(mine).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&l) else {
            continue;
        };
        let last = v.get("exit").is_some() || v.get("denied").is_some();
        frames.push(v);
        if last {
            break;
        }
    }
    assert!(started.elapsed() < Duration::from_secs(20), "runner hung");
    drop(child.wait());
    frames
}

fn uid_of_test_process() -> u32 {
    // Same uid as the runner: found out via `id -u` with no libc dependency.
    let o = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
}

fn stdout_of(frames: &[Value]) -> String {
    frames
        .iter()
        .filter(|f| f["stream"] == "stdout")
        .map(|f| f["data"].as_str().unwrap())
        .collect()
}

fn done(frames: &[Value]) -> &Value {
    frames.last().expect("final frame")
}

#[test]
fn runner_runs_command_in_project_and_streams_in_order() {
    let r = rig();
    let uid = uid_of_test_process();
    let f = ask(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "echo one; echo two >&2; exit 3"}),
    );
    assert_eq!(stdout_of(&f), "one\n");
    assert!(
        f.iter()
            .any(|x| x["stream"] == "stderr" && x["data"] == "two\n"),
        "{f:?}"
    );
    assert_eq!(done(&f)["exit"], 3, "{f:?}");
    assert_eq!(
        (done(&f)["timeout"].clone(), done(&f)["truncated"].clone()),
        (json!(false), json!(false))
    );
    let seqs: Vec<u64> = f
        .iter()
        .filter(|x| x.get("seq").is_some())
        .map(|x| x["seq"].as_u64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seq in order: {seqs:?}"
    );
    let g = ask(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "pwd; : > created.txt"}),
    );
    assert!(r.project.join("created.txt").exists());
    assert_eq!(stdout_of(&g).trim(), r.project.to_string_lossy());
}

#[test]
fn runner_peercred_other_uid_rejected() {
    let r = rig();
    let uid = uid_of_test_process();
    let f = ask(
        uid + 1,
        &[&r.project],
        &json!({"cwd": r.project, "command": "echo secret > leaked.txt"}),
    );
    assert_eq!(done(&f)["denied"], "peer", "{f:?}");
    assert!(
        !r.project.join("leaked.txt").exists(),
        "command did not run"
    );
}

#[test]
fn runner_dir_outside_allowlist_rejected() {
    let r = rig();
    let uid = uid_of_test_process();
    let outside = r.dir.path().canonicalize().unwrap().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, r.project.join("link")).unwrap();
    std::fs::create_dir_all(r.project.join(".ssh")).unwrap();
    std::fs::create_dir_all(r.project.join(".agentos")).unwrap();
    let cases: Vec<(PathBuf, &str)> = vec![
        (PathBuf::from("/etc"), "dir"),
        (PathBuf::from("/"), "dir"),
        (outside.clone(), "dir"),
        (r.project.join("../outside"), "dir"),
        (r.project.join("link"), "dir"),
        (r.project.join(".ssh"), "forbidden"),
        (r.project.join(".agentos"), "forbidden"),
        (PathBuf::from("/run/user/1000"), "dir"),
        (r.project.join("no-such-dir"), "dir"),
    ];
    for (cwd, why) in cases {
        let f = ask(
            uid,
            &[&r.project],
            &json!({"cwd": cwd, "command": "echo hi > proof.txt"}),
        );
        assert_eq!(done(&f)["denied"], why, "{cwd:?}: {f:?}");
    }
    assert!(!outside.join("proof.txt").exists() && !r.project.join("proof.txt").exists());
}

#[test]
fn runner_extra_params_rejected() {
    let r = rig();
    let uid = uid_of_test_process();
    for extra in [
        ("p", json!("x")),
        ("-p", json!("x")),
        ("systemd_run_args", json!(["--uid=0"])),
        ("env", json!({"A": "b"})),
        ("uid", json!(0)),
    ] {
        let mut req = json!({"cwd": r.project, "command": "true"});
        req[extra.0] = extra.1;
        let f = ask(uid, &[&r.project], &req);
        assert_eq!(done(&f)["denied"], "bad_request", "{}: {f:?}", extra.0);
    }
    for bad in [
        json!({"cwd": r.project}),
        json!({"command": "true"}),
        json!({"cwd": r.project, "command": "true", "timeout_secs": 0}),
        json!({"cwd": r.project, "command": "true", "timeout_secs": 9999}),
        json!({"cwd": r.project, "command": ""}),
    ] {
        let f = ask(uid, &[&r.project], &bad);
        assert_eq!(done(&f)["denied"], "bad_request", "{bad}: {f:?}");
    }
}

#[test]
fn runner_environment_is_an_allowlist() {
    let r = rig();
    let uid = uid_of_test_process();
    let f = ask(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "env | cut -d= -f1 | sort"}),
    );
    let names: Vec<String> = stdout_of(&f).lines().map(str::to_string).collect();
    let allowed = ["PATH", "LANG", "HOME", "PWD", "SHLVL", "_", "OLDPWD"];
    assert!(
        names.iter().all(|n| allowed.contains(&n.as_str())),
        "extra variables: {names:?}"
    );
    assert!(names.contains(&"PATH".to_string()));
}

#[test]
fn agent_cannot_find_gui_token() {
    // Worst case: the GUI token and the core socket path are in the runner process environment. The agent command looks for the token in its own
    // environment and in `/proc/self/environ`; the command environment is an allowlist (`runner_environment_is_an_allowlist`), so there is no token.
    // Descriptors are not checked here (platform-dependent); the GUI token via fd is `gui_token_via_fd_not_in_environ` (Linux).
    // The value is assembled at run time (the source has no token literal: the secret guard catches it).
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let token = &format!("agt_{nanos:032x}{:016x}", std::process::id());
    let r = rig();
    let uid = uid_of_test_process();
    let f = ask_env(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "env; cat /proc/self/environ 2>/dev/null | tr '\\0' '\\n'; echo END"}),
        &[
            ("AGENTD_TOKEN", token),
            ("AGENTD_TOKEN_FD", "3"),
            ("AGENTD_GUI_TOKEN", token),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ],
    );
    let out = stdout_of(&f);
    assert!(out.contains("END"), "command ran: {out}");
    assert!(
        !out.contains("agt_") && !out.contains(&token[4..]),
        "token visible to the agent command: {out}"
    );
    for name in ["AGENTD_TOKEN", "AGENTD_TOKEN_FD", "XDG_RUNTIME_DIR"] {
        assert!(!out.contains(name), "{name} visible to the agent command");
    }
}

#[test]
fn runner_timeout_kills_the_whole_group_and_output_is_capped() {
    let r = rig();
    let uid = uid_of_test_process();
    let t = Instant::now();
    let f = ask(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "sleep 30 & sleep 30; echo wont-arrive", "timeout_secs": 1}),
    );
    assert_eq!(done(&f)["timeout"], true, "{f:?}");
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    let g = ask(
        uid,
        &[&r.project],
        &json!({"cwd": r.project, "command": "yes x | head -c 500000", "max_output": 1000}),
    );
    assert_eq!(done(&g)["truncated"], true);
    assert!(stdout_of(&g).len() <= 1000, "{}", stdout_of(&g).len());
}

#[test]
fn runner_garbage_request_is_denied_not_fatal() {
    let uid = uid_of_test_process();
    let r = rig();
    let (mine, theirs) = UnixStream::pair().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentd"))
        .args(["runner", "--allow-uid", &uid.to_string(), "--allow-dir"])
        .arg(&r.project)
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = mine.try_clone().unwrap();
    writeln!(w, "this is not json").unwrap();
    let lines: Vec<String> = BufReader::new(mine).lines().map_while(Result::ok).collect();
    assert!(lines.iter().any(|l| l.contains("bad_request")), "{lines:?}");
    assert!(child.wait().unwrap().success());
}
