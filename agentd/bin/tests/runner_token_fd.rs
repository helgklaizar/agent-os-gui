//! A7-02: a `kind = agent` token reaches the `shell` command only via a descriptor. The client sends the request line together with
//! a descriptor (`SCM_RIGHTS`) and the `token_fd` flag; the runner leaves the descriptor to the command and puts its number in
//! `AGENTD_TOKEN_FD`. The value does not reach the environment or arguments. No flag or no descriptor is a refusal.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TOKEN: &str = "tok-0123456789abcdef-test-value";
/// Reads the token from the descriptor without exposing it in arguments: `eval` expands only the number.
const READ_FD: &str = "eval \"cat <&$AGENTD_TOKEN_FD\"";

fn my_uid() -> String {
    let o = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// A descriptor with content: a pipe with its write end closed (on Linux the core sends a `memfd`, the runner does not care).
fn secret_fd(contents: &str) -> OwnedFd {
    let (rd, mut wr) = std::io::pipe().unwrap();
    wr.write_all(contents.as_bytes()).unwrap();
    drop(wr);
    OwnedFd::from(rd)
}

/// Starts the runner with the accepted connection on standard input; the request goes with a descriptor or without.
fn ask(project: &Path, req: &Value, fd: Option<OwnedFd>) -> Vec<Value> {
    let (mine, theirs) = UnixStream::pair().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentd"))
        .arg("runner")
        .args(["--allow-uid", &my_uid()])
        .arg("--allow-dir")
        .arg(project)
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let line = format!("{req}\n");
    match &fd {
        Some(fd) => agentd_sys::fdpass::send_fd(&mine, line.as_bytes(), fd.as_fd()).unwrap(),
        None => (&mine).write_all(line.as_bytes()).unwrap(),
    }
    drop(fd);
    let started = Instant::now();
    mine.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut frames = Vec::new();
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

fn project() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().canonicalize().unwrap().join("project");
    std::fs::create_dir_all(&p).unwrap();
    (dir, p)
}

fn stdout_of(frames: &[Value]) -> String {
    frames
        .iter()
        .filter(|f| f["stream"] == "stdout")
        .map(|f| f["data"].as_str().unwrap())
        .collect()
}

fn denied(frames: &[Value]) -> Option<&str> {
    frames.iter().find_map(|f| f["denied"].as_str())
}

#[test]
fn runner_token_fd_reaches_the_command() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": READ_FD, "token_fd": true});
    let frames = ask(&p, &req, Some(secret_fd(TOKEN)));
    assert_eq!(denied(&frames), None, "{frames:?}");
    assert_eq!(stdout_of(&frames), TOKEN, "{frames:?}");
    assert_eq!(frames.last().unwrap()["exit"], 0);
}

/// `dash` (`/bin/sh` on Ubuntu) understands a single digit in `<&N`: the token descriptor number must be 3 to 9.
#[test]
fn runner_token_fd_number_is_a_single_digit() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": "echo $AGENTD_TOKEN_FD", "token_fd": true});
    let frames = ask(&p, &req, Some(secret_fd(TOKEN)));
    let n: u32 = stdout_of(&frames)
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("descriptor number is not a number: {frames:?}"));
    assert!((3..=9).contains(&n), "number {n}: {frames:?}");
}

#[test]
fn runner_token_never_in_environment_or_arguments() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": "env; ps -o args= -p $$", "token_fd": true});
    let frames = ask(&p, &req, Some(secret_fd(TOKEN)));
    let out = stdout_of(&frames);
    assert!(out.contains("AGENTD_TOKEN_FD="), "descriptor number: {out}");
    assert!(!out.contains(TOKEN), "token value leaked: {out}");
    assert!(!out.contains("AGENTD_TOKEN="), "{out}");
}

#[test]
fn runner_without_token_has_no_fd_variable() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": "echo ${AGENTD_TOKEN_FD-unset}"});
    let frames = ask(&p, &req, None);
    assert_eq!(stdout_of(&frames), "unset\n", "{frames:?}");
}

#[test]
fn runner_token_flag_without_descriptor_is_denied() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": "touch ran.txt", "token_fd": true});
    let frames = ask(&p, &req, None);
    assert_eq!(denied(&frames), Some("bad_request"), "{frames:?}");
    assert!(frames.iter().all(|f| f.get("started").is_none()));
    assert!(!p.join("ran.txt").exists(), "command did not run");
}

#[test]
fn runner_descriptor_without_flag_is_denied() {
    let (_d, p) = project();
    let req = json!({"cwd": p, "command": "touch ran.txt"});
    let frames = ask(&p, &req, Some(secret_fd(TOKEN)));
    assert_eq!(denied(&frames), Some("bad_request"), "{frames:?}");
    assert!(!p.join("ran.txt").exists(), "command did not run");
}

#[test]
fn runner_token_flag_must_be_boolean_true() {
    let (_d, p) = project();
    for bad in [json!("yes"), json!(1), json!(null)] {
        let req = json!({"cwd": p, "command": "touch ran.txt", "token_fd": bad});
        let frames = ask(&p, &req, Some(secret_fd(TOKEN)));
        assert_eq!(denied(&frames), Some("bad_request"), "{bad}: {frames:?}");
    }
    assert!(!p.join("ran.txt").exists());
}
