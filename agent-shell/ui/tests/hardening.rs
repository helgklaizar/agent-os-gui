//! Process hardening: `GDK_DEBUG=no-portals` and `PR_SET_DUMPABLE=0`.
#![cfg(target_os = "linux")]

use std::os::unix::fs::MetadataExt;

use ui::hardening::{gdk_debug_value, is_dumpable, set_not_dumpable};

#[test]
fn hardening_gdk_debug_adds_no_portals_and_keeps_flags() {
    assert_eq!(gdk_debug_value(None), "no-portals");
    assert_eq!(gdk_debug_value(Some("")), "no-portals");
    assert_eq!(
        gdk_debug_value(Some("vulkan-disable")),
        "vulkan-disable,no-portals"
    );
    assert_eq!(gdk_debug_value(Some("no-portals")), "no-portals");
    assert_eq!(
        gdk_debug_value(Some("opengl,no-portals,events")),
        "opengl,no-portals,events"
    );
    // glib accepts a colon separator, the value stays readable
    assert_eq!(
        gdk_debug_value(Some("opengl:events")),
        "opengl:events,no-portals"
    );
}

#[test]
fn hardening_not_dumpable_closes_proc_for_same_uid() {
    assert!(
        is_dumpable(),
        "the test process is ordinary, dumpable by default"
    );
    let me = std::fs::metadata("/proc/self/status").expect("proc").uid();
    set_not_dumpable().expect("prctl");
    assert!(!is_dumpable());
    // independent observation: for a non-dumpable process the /proc/self files belong to root
    let owner = std::fs::metadata("/proc/self/environ").expect("proc").uid();
    assert_eq!(owner, 0, "environ must belong to root, not uid {me}");
}
