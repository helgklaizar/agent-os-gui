//! A6 (browser client): `agentd token issue|list|revoke` issues external-client tokens from the command line,
//! with a live core via the service socket, with a stopped one directly in the DB.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Rig {
    _dir: tempfile::TempDir,
    db: PathBuf,
    sock: PathBuf,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    Rig {
        db: dir.path().join("agentos/sessions.db"),
        sock: dir.path().join("agentd.sock"),
        _dir: dir,
    }
}

fn agentd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env_remove("AGENTD_TCP");
    c
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

fn serve(r: &Rig) -> Daemon {
    let daemon = Daemon(
        agentd(&[
            "serve",
            "--db",
            r.db.to_str().unwrap(),
            "--socket",
            r.sock.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if get(&r.sock, "/v1/health", None).contains("\"status\":\"ok\"") {
            return daemon;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("agentd did not become ready");
}

fn get(sock: &Path, path: &str, token: Option<&str>) -> String {
    let Ok(mut s) = UnixStream::connect(sock) else {
        return String::new();
    };
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    drop(s.read_to_string(&mut out));
    out
}

fn status(resp: &str) -> &str {
    resp.split_whitespace().nth(1).unwrap_or("")
}

/// The `token` subcommand: the core is found by `--socket` (service socket nearby), otherwise the DB by `--db`.
fn token(r: &Rig, args: &[&str]) -> Output {
    let mut a = vec!["token"];
    a.extend_from_slice(args);
    a.extend_from_slice(&[
        "--db",
        r.db.to_str().unwrap(),
        "--socket",
        r.sock.to_str().unwrap(),
    ]);
    agentd(&a)
        .stdin(Stdio::null())
        .output()
        .expect("agentd token starts")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

#[test]
fn token_cli_issue_list_revoke_against_live_core() {
    let r = rig();
    let _d = serve(&r);
    let o = token(&r, &["issue", "--name", "browser", "--scope", "chat"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let value = out(&o);
    assert!(
        !value.is_empty() && !value.contains(char::is_whitespace),
        "one line is the token value"
    );
    assert_eq!(status(&get(&r.sock, "/v1/sessions", Some(&value))), "200");
    assert_eq!(
        status(&get(&r.sock, "/v1/sessions", Some("foreign"))),
        "401"
    );

    let l = token(&r, &["list"]);
    let list = out(&l);
    assert!(
        list.contains("browser") && list.contains("chat") && list.contains("external"),
        "{list}"
    );
    assert!(
        !list.contains(&value),
        "token value is not shown in the list"
    );

    let id = list
        .lines()
        .find(|l| l.contains("browser"))
        .and_then(|l| l.split_whitespace().next())
        .unwrap()
        .to_string();
    let rv = token(&r, &["revoke", "--id", &id]);
    assert!(
        rv.status.success(),
        "{}",
        String::from_utf8_lossy(&rv.stderr)
    );
    assert_eq!(
        status(&get(&r.sock, "/v1/sessions", Some(&value))),
        "401",
        "revoked token is not accepted"
    );
    let again = token(&r, &["revoke", "--id", &id]);
    assert!(again.status.success(), "repeated revoke is harmless");
    let missing = token(&r, &["revoke", "--id", "9999"]);
    assert!(!missing.status.success(), "no such token: error");
}

#[test]
fn token_cli_rejects_bad_input_without_printing_a_token() {
    let r = rig();
    let _d = serve(&r);
    for args in [
        vec!["issue", "--name", "x", "--scope", "admin"],
        vec!["issue", "--name", "x", "--scope", "root"],
        vec!["issue", "--scope", "read"],
        vec![
            "issue",
            "--name",
            "x",
            "--scope",
            "read",
            "--ttl-days",
            "900",
        ],
        vec!["revoke"],
        vec!["unknown"],
    ] {
        let o = token(&r, &args);
        assert!(!o.status.success(), "{args:?} must be rejected");
        assert!(out(&o).is_empty(), "{args:?}: token not printed");
    }
}

#[test]
fn token_cli_works_offline_directly_on_the_database() {
    let r = rig();
    let o = token(
        &r,
        &[
            "issue",
            "--name",
            "offline",
            "--scope",
            "read",
            "--ttl-days",
            "30",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let value = out(&o);
    assert!(out(&token(&r, &["list"])).contains("offline"));
    let _d = serve(&r);
    assert_eq!(
        status(&get(&r.sock, "/v1/sessions", Some(&value))),
        "200",
        "token issued without the core is accepted"
    );
    let post = {
        let mut s = UnixStream::connect(&r.sock).unwrap();
        let req = format!(
            "POST /v1/sessions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {value}\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{{\"name\":\"x\"}}"
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut o = String::new();
        drop(s.read_to_string(&mut o));
        o
    };
    assert_eq!(status(&post), "403", "read does not create sessions");
}

#[test]
fn token_cli_admin_socket_is_private() {
    let r = rig();
    let _d = serve(&r);
    let admin = r.sock.with_file_name("agentd-admin.sock");
    let mode = std::fs::metadata(&admin)
        .expect("service socket created")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn token_cli_decide_issues_web_token_with_grant() {
    let r = rig();
    let _d = serve(&r);
    let o = token(
        &r,
        &["issue", "--name", "browser", "--scope", "chat", "--decide"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let value = out(&o);
    assert!(value.starts_with("agt_"), "token printed once");
    let list = out(&token(&r, &["list"]));
    let line0 = list.lines().find(|l| l.contains("browser")).unwrap();
    assert!(line0.contains("\tweb\tdecide\t"), "{line0}");
    // Default lifetime is 7 days: the expiry column is filled, not a dash.
    let line = list.lines().find(|l| l.contains("browser")).unwrap();
    assert!(!line.split('\t').nth(4).unwrap().starts_with('-'), "{line}");
    // A plain token without `--decide` gets no mark.
    let plain = token(&r, &["issue", "--name", "plain", "--scope", "chat"]);
    assert!(plain.status.success());
    let list = out(&token(&r, &["list"]));
    let plain_line = list.lines().find(|l| l.contains("plain")).unwrap();
    assert!(plain_line.contains("\texternal\tchat\t"), "{plain_line}");
    // Limits: only chat and lifetime up to 30 days.
    for args in [
        vec!["issue", "--name", "x", "--scope", "read", "--decide"],
        vec![
            "issue",
            "--name",
            "x",
            "--scope",
            "chat",
            "--decide",
            "--ttl-days",
            "31",
        ],
        vec![
            "issue",
            "--name",
            "x",
            "--scope",
            "chat",
            "--decide",
            "--ttl-days",
            "0",
        ],
    ] {
        let o = token(&r, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(out(&o).is_empty(), "{args:?}: token not printed");
    }
}
