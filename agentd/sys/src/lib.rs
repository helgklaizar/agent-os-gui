//! System calls of `agentd` behind safe wrappers. The only core crate where `unsafe` is allowed
//! by design; currently there are no `unsafe` blocks: calls go through the safe `rustix` API. If `unsafe` appears,
//! lift `forbid` in this crate and document each block (invariant, review).

pub mod fdpass;
pub mod metrics;
pub mod paths;
pub mod proc;
pub mod sysfs;

use std::path::Path;

/// Free space on the filesystem containing `path`, in bytes (available to an unprivileged user).
pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let st = rustix::fs::statvfs(path)?;
    Ok(st.f_bavail.saturating_mul(st.f_frsize))
}

/// Kills process group `pgid` with `KILL` (runner: the command and all its descendants). A missing group is not an error.
pub fn kill_group(pgid: i32) {
    if let Some(p) = rustix::process::Pid::from_raw(pgid) {
        let _ = rustix::process::kill_process_group(p, rustix::process::Signal::KILL);
    }
}

/// Effective uid of the process.
#[must_use]
pub fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_bytes_of_tempdir_is_positive() {
        let d = tempfile::tempdir().expect("tempdir");
        assert!(free_bytes(d.path()).expect("statvfs") > 0);
    }

    #[test]
    fn free_bytes_missing_path_is_error() {
        assert!(free_bytes(Path::new("/nonexistent-agentd-path")).is_err());
    }
}
