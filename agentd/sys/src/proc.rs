//! Process information from `/proc` (read-only): ancestors, start time, session unit. Needed by `gui-launch`.

use std::io;

/// Process from `/proc/<pid>/stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: i32,
    pub comm: String,
    pub ppid: i32,
    /// Start time in ticks since boot: together with the pid it distinguishes a process from another with the same pid.
    pub start: u64,
}

/// Parses a `/proc/<pid>/stat` line: `comm` is in parentheses and may contain spaces and parentheses, so
/// the fields after it are counted from the last `)`.
pub fn parse_stat(pid: i32, stat: &str) -> Option<ProcInfo> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = stat[open + 1..close].to_string();
    let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    // rest[0] is state, rest[1] is ppid, rest[19] is starttime (field 22 per proc(5)).
    Some(ProcInfo {
        pid,
        comm,
        ppid: rest.get(1)?.parse().ok()?,
        start: rest.get(19)?.parse().ok()?,
    })
}

pub fn proc_info(pid: i32) -> io::Result<ProcInfo> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    parse_stat(pid, &stat)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stat not parsed"))
}

/// Ancestors of a process from parent to root (`pid` itself is not in the list). A broken chain (process gone) ends the list.
#[must_use]
pub fn ancestors(pid: i32) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let mut cur = match proc_info(pid) {
        Ok(i) => i.ppid,
        Err(_) => return out,
    };
    while cur > 1 && out.len() < 64 {
        let Ok(info) = proc_info(cur) else { break };
        cur = info.ppid;
        out.push(info);
    }
    out
}

/// Whether that very process is alive (pid and start time match).
#[must_use]
pub fn is_alive(pid: i32, start: u64) -> bool {
    proc_info(pid).is_ok_and(|i| i.start == start)
}

/// `kernel.yama.ptrace_scope`; `None` means the file is absent (Yama not enabled or not Linux).
#[must_use]
pub fn ptrace_scope() -> Option<u8> {
    std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comm_with_spaces_and_parens() {
        let stat = "123 (my (weird) proc) S 45 123 123 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 10 18446744073709551615";
        let i = parse_stat(123, stat).unwrap();
        assert_eq!(
            (i.comm.as_str(), i.ppid, i.start),
            ("my (weird) proc", 45, 987_654)
        );
        assert!(parse_stat(1, "garbage").is_none());
        assert!(parse_stat(1, "1 (x) S 2").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn own_process_has_parent_and_is_alive() {
        let me = i32::try_from(std::process::id()).unwrap();
        let info = proc_info(me).unwrap();
        assert!(is_alive(me, info.start));
        assert!(!is_alive(me, info.start + 1));
        assert!(!ancestors(me).is_empty());
        assert!(ancestors(i32::MAX).is_empty());
    }
}
