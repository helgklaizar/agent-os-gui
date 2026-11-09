//! A6-06 on real processes (A-core-100): the core starts an `ssh` tunnel with a real `ssh` to a real `sshd` (its own, on a
//! high port, with one-time keys, as the current user), forwards a port and checks data end to end;
//! a killed `ssh` is restarted, disabling stops the process. The same argument vector as in production
//! (`-N -o ExitOnForwardFailure=yes -o BatchMode=yes -- <address>`); the wrapper (`AGENTD_TUNNEL_SSH`, only in the
//! `test-support` build) only adds the port, key and `-L` forward. Linux only; needs `ssh`, `ssh-keygen` and `/usr/sbin/sshd`.
#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::Env;
use serde_json::json;

struct Sshd {
    child: Child,
    port: u16,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn keygen(path: &Path) {
    let st = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .expect("ssh-keygen");
    assert!(st.success());
}

fn user() -> String {
    let o = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

/// A real `sshd` without root: its own host key, login only with the `authorized` key.
fn start_sshd(dir: &Path, authorized: &Path) -> Sshd {
    let host = dir.join("hostkey");
    keygen(&host);
    let port = free_port();
    let cfg = dir.join("sshd_config");
    std::fs::write(
        &cfg,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\n\
             UsePAM no\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n\
             PubkeyAuthentication yes\nAllowTcpForwarding yes\nLogLevel ERROR\n",
            host.display(),
            dir.join("sshd.pid").display(),
            authorized.display()
        ),
    )
    .unwrap();
    let child = Command::new("/usr/sbin/sshd")
        .args(["-D", "-e", "-f"])
        .arg(&cfg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sshd");
    let end = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < end, "sshd did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Sshd { child, port }
}

/// Echo server: whatever arrives is sent back; the forward target.
fn echo_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in l.incoming().map_while(Result::ok) {
            std::thread::spawn(move || {
                let mut b = [0u8; 256];
                while let Ok(n) = s.read(&mut b) {
                    if n == 0 || s.write_all(&b[..n]).is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

/// Wrapper script around the real `ssh`: sshd port, key and forward `lport` -> echo server.
fn wrapper(dir: &Path, name: &str, sshd: u16, key: &Path, lport: u16, target: u16) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nexec /usr/bin/ssh -F /dev/null -i {} -p {sshd} -o IdentitiesOnly=yes \
             -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
             -L {lport}:127.0.0.1:{target} \"$@\"\n",
            key.display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(&p).status().unwrap();
    p
}

fn echo_through(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    if s.write_all(b"ping-through-ssh").is_err() {
        return false;
    }
    let mut b = [0u8; 16];
    s.read_exact(&mut b).is_ok() && &b == b"ping-through-ssh"
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn status_of(e: &Env, id: i64) -> String {
    let (_, list) = e.admin.get("/tunnels").await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn wait_status(e: &Env, id: i64, want: &str) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if status_of(e, id).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("status {want} not reached, now {}", status_of(e, id).await);
}

fn kill_ssh(lport: u16) {
    drop(
        Command::new("pkill")
            .args(["-KILL", "-f", "--", &format!("-L {lport}:127.0.0.1")])
            .status(),
    );
}

fn ssh_running(lport: u16) -> bool {
    Command::new("pgrep")
        .args(["-f", "--", &format!("-L {lport}:127.0.0.1")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

async fn create(e: &Env, addr: &str) -> i64 {
    e.admin
        .make(
            "/tunnels",
            json!({"type": "ssh", "addr": addr, "enabled": true}),
        )
        .await["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn real_ssh_tunnel_carries_data_restarts_after_kill_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let client_key = dir.path().join("client");
    keygen(&client_key);
    let sshd = start_sshd(dir.path(), &dir.path().join("client.pub"));
    let target = echo_server();
    let lport = free_port();
    let w = wrapper(
        dir.path(),
        "ssh-real.sh",
        sshd.port,
        &client_key,
        lport,
        target,
    );
    let e = Env::start_with_env(vec![(
        "AGENTD_TUNNEL_SSH".into(),
        w.to_string_lossy().into_owned(),
    )])
    .await;
    let id = create(&e, &format!("{}@127.0.0.1", user())).await;
    wait_status(&e, id, "up").await;
    wait_until("data flows through the real ssh", || echo_through(lport)).await;
    // Killed ssh: the core starts it again (1 s pause), the forward comes back.
    kill_ssh(lport);
    wait_until("ssh killed", || !echo_through(lport)).await;
    wait_until("ssh restarted by the core", || echo_through(lport)).await;
    wait_status(&e, id, "up").await;
    // Disabling: the process is stopped, the port is closed.
    assert_eq!(
        e.admin
            .patch(&format!("/tunnels/{id}"), json!({"enabled": false}))
            .await
            .0,
        200
    );
    wait_status(&e, id, "down").await;
    wait_until("ssh stopped", || !ssh_running(lport)).await;
    assert!(!echo_through(lport), "no forward after stop");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_ssh_with_unauthorized_key_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = dir.path().join("allowed");
    keygen(&allowed);
    let stranger = dir.path().join("stranger");
    keygen(&stranger);
    let sshd = start_sshd(dir.path(), &dir.path().join("allowed.pub"));
    let lport = free_port();
    let w = wrapper(
        dir.path(),
        "ssh-bad.sh",
        sshd.port,
        &stranger,
        lport,
        echo_server(),
    );
    let e = Env::start_with_env(vec![(
        "AGENTD_TUNNEL_SSH".into(),
        w.to_string_lossy().into_owned(),
    )])
    .await;
    let id = create(&e, &format!("{}@127.0.0.1", user())).await;
    wait_status(&e, id, "error").await;
    assert!(!echo_through(lport), "no forward without login");
    let (_, ev) = e.read.get("/events?kind=msg&sub=sys").await;
    assert!(
        ev["items"].as_array().unwrap().iter().any(|x| x["text"]
            .as_str()
            .unwrap()
            .contains("Tunnel")
            && x["level"] == "warn"),
        "status change is an event: {ev}"
    );
}
