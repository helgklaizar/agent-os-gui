//! Process hardening: `PR_SET_DUMPABLE=0` and `GDK_DEBUG=no-portals`.
//!
//! `PR_SET_DUMPABLE=0` closes `/proc/PID/{environ,fd,mem}` to a neighbouring process of the same uid (the GUI token lives in memory).
//! `GDK_DEBUG=no-portals` removes the 25-50 second `xdg-desktop-portal` wait at startup under cage.
//!
//! The crate's only `unsafe`: the `prctl` call and setting an environment variable before threads are created.
#![allow(unsafe_code)]

use std::io;

const PR_SET_DUMPABLE: i32 = 4;
const PR_GET_DUMPABLE: i32 = 3;

unsafe extern "C" {
    // SAFETY (for all calls below): prototype from <sys/prctl.h>, arguments are plain integers without pointers.
    fn prctl(option: i32, ...) -> i32;
}

/// `GDK_DEBUG` value with `no-portals`; other flags are kept as they are.
pub fn gdk_debug_value(existing: Option<&str>) -> String {
    match existing.map(str::trim).filter(|v| !v.is_empty()) {
        None => "no-portals".to_string(),
        Some(v) if v.split([',', ':', ';', ' ']).any(|f| f == "no-portals") => v.to_string(),
        Some(v) => format!("{v},no-portals"),
    }
}

/// `prctl(PR_GET_DUMPABLE)`.
pub fn is_dumpable() -> bool {
    // SAFETY: PR_GET_DUMPABLE takes no pointers and does not change state.
    unsafe { prctl(PR_GET_DUMPABLE, 0u64, 0u64, 0u64, 0u64) == 1 }
}

/// `prctl(PR_SET_DUMPABLE, 0)`.
pub fn set_not_dumpable() -> io::Result<()> {
    // SAFETY: PR_SET_DUMPABLE with zero takes no pointers; the process only loses the right to be dumped.
    let rc = unsafe { prctl(PR_SET_DUMPABLE, 0u64, 0u64, 0u64, 0u64) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Everything at once. Call first thing in `main`, before threads and before `gtk::init`.
pub fn apply() -> io::Result<()> {
    let value = gdk_debug_value(std::env::var("GDK_DEBUG").ok().as_deref());
    // SAFETY: called from `main` before threads are created; there are no other readers of the environment yet.
    unsafe { std::env::set_var("GDK_DEBUG", value) };
    set_not_dumpable()
}
