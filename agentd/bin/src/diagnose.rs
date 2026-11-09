//! `agentd diagnose` (A7-10, diagnostics spec 4.2-4.3): a report without personal data. Only versions, environment and state: paths, names
//! and tokens are not printed; a line that cannot be reliably sanitised is replaced with `<dropped: unsafe>`; report <= 256 KB.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

const MAX_BYTES: usize = 256 * 1024;
const DROPPED: &str = "<dropped: unsafe>";

/// Suspicious personal data markers: an absolute path, a home directory, an `agt_` token, credentials in an address.
fn unsafe_line(line: &str, secrets: &[String]) -> bool {
    line.contains("agt_")
        || line.contains("/home/")
        || line.contains("/Users/")
        || line.contains("://") && line.contains('@')
        || secrets
            .iter()
            .any(|s| !s.is_empty() && line.contains(s.as_str()))
}

/// Blacklist: each line is either clean or replaced.
pub fn sanitize(lines: &[String], secrets: &[String]) -> Vec<String> {
    let mut out: Vec<String> = lines
        .iter()
        .map(|l| {
            if unsafe_line(l, secrets) {
                DROPPED.to_string()
            } else {
                l.clone()
            }
        })
        .collect();
    let mut total = 0;
    out.retain(|l| {
        total += l.len() + 1;
        total <= MAX_BYTES
    });
    out
}

fn health(socket: &Path) -> Option<String> {
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .ok()?;
    s.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    let body = out.split("\r\n\r\n").nth(1)?;
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .map(|v| {
            format!(
                "{} (api {}.{}, capabilities {})",
                v["status"].as_str().unwrap_or("?"),
                v["api"]["major"],
                v["api"]["minor"],
                v["capabilities"]
            )
        })
}

pub fn report(db: &Path, socket: &Path) -> Vec<String> {
    let mut l = vec![
        format!("agentd {}", env!("CARGO_PKG_VERSION")),
        format!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH),
        match agentd_sys::proc::ptrace_scope() {
            Some(v) => format!("ptrace_scope: {v}"),
            None => "ptrace_scope: unknown".to_string(),
        },
        // Disk encryption state is reliably determined only by the `agent-diag` package with block device read rights.
        "disk_encryption: unknown".to_string(),
    ];
    l.push(match health(socket) {
        Some(h) => format!("health: {h}"),
        None => "health: unavailable".to_string(),
    });
    l.push(if db.exists() {
        match agentd_db::check_migrations(db, agentd_db::MIGRATIONS) {
            Ok(c) => format!("db: user_version {} of {}", c.db_version, c.known_version),
            Err(_) => "db: unreadable".to_string(),
        }
    } else {
        "db: absent".to_string()
    });
    l.push(crate::heartbeat::report_line(
        crate::heartbeat::from_env(&|k| std::env::var(k).ok()).as_ref(),
    ));
    l.push("memory_source: requires a token, not collected".to_string());
    l.push("log_tail: not collected (journald)".to_string());
    sanitize(&l, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_lines_are_replaced() {
        let s = |x: &str| x.to_string();
        let out = sanitize(
            &[
                s("ok: 1"),
                s("path /home/user/x"),
                s("token agt_abc"),
                s("url https://u:p@h/"),
                s("mine SECRET-VALUE"),
            ],
            &[s("SECRET-VALUE")],
        );
        assert_eq!(out, ["ok: 1", DROPPED, DROPPED, DROPPED, DROPPED]);
    }

    #[test]
    fn report_is_capped() {
        let big: Vec<String> = (0..100_000).map(|i| format!("line {i}")).collect();
        let out = sanitize(&big, &[]);
        assert!(out.iter().map(|l| l.len() + 1).sum::<usize>() <= MAX_BYTES);
    }
}
