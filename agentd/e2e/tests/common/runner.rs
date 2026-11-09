//! Fake socket activation for the agent runner (A7-02): a `runner.sock` socket next to the core socket; for each connection
//! a real `agentd runner` is started with the accepted connection on stdin (like `Accept=yes`).
#![allow(dead_code)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{Env, agentd_bin};

pub struct RunnerGuard {
    stop: Arc<AtomicBool>,
    sock: PathBuf,
    thread: Option<JoinHandle<()>>,
}

fn my_uid() -> String {
    let o = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

impl Env {
    /// Runner socket path that the core receives in `AGENTD_RUNNER_SOCKET`.
    pub fn runner_sock(&self) -> PathBuf {
        self.sock.with_file_name("runner.sock")
    }

    /// Starts the "runner" for the `dirs` directories; `drop` stops it (the socket disappears, so the runner is "stopped").
    pub fn start_runner(&self, dirs: &[&Path]) -> RunnerGuard {
        let sock = self.runner_sock();
        drop(std::fs::remove_file(&sock));
        let listener = UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let dirs: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
        let thread = std::thread::spawn(move || {
            let mut kids = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let mut c = Command::new(agentd_bin());
                        c.arg("runner").args(["--allow-uid", &my_uid()]);
                        for d in &dirs {
                            c.arg("--allow-dir").arg(d);
                        }
                        conn.set_nonblocking(false).unwrap();
                        let kid = c
                            .stdin(Stdio::from(OwnedFd::from(conn)))
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()
                            .unwrap();
                        kids.push(kid);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            for mut k in kids {
                drop(k.kill());
                drop(k.wait());
            }
        });
        RunnerGuard {
            stop,
            sock,
            thread: Some(thread),
        }
    }
}

impl Drop for RunnerGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            drop(t.join());
        }
        drop(std::fs::remove_file(&self.sock));
    }
}
