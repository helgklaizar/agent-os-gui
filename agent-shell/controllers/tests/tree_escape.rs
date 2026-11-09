//! S3-10: hostile input in tree names and performance with 500 sessions.

mod tree_common;

use std::time::Instant;

use controllers::Controller;
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::testkit::evil_strings;
use controllers::tree_types::TreeFrame;
use tree_common::*;

// ---------- hostile input ----------

#[test]
fn escape_session_name() {
    let evil = "<img src=x onerror=alert(1)>";
    let mut c = SessionTreeController::new();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, evil)],
        },
    );
    let v = c.view();
    assert_eq!(v.rows[0].name, evil, "shown verbatim");
    assert!(v.rows[0].a11y_name.contains(evil));
}

#[test]
fn escape_folder_name_in_attr() {
    let evil = "\" onmouseover=\"x";
    let mut c = SessionTreeController::new();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![folder(1, None, evil, None)],
            sessions: vec![],
        },
    );
    assert_eq!(c.view().rows[0].name, evil);
}

#[test]
fn escape_all_evil_strings_literal_or_truncated() {
    for evil in evil_strings() {
        let mut c = SessionTreeController::new();
        send(
            &mut c,
            TreeMsg::Snapshot {
                folders: vec![folder(1, None, &evil, None)],
                sessions: vec![session(2, Some(1), &evil)],
            },
        );
        for r in c.view().rows {
            let shown = r.name.strip_suffix('…').unwrap_or(&r.name);
            assert!(
                evil.starts_with(shown),
                "name shown verbatim (truncated): {:?}",
                r.name
            );
            assert!(r.name.chars().count() <= 121);
        }
    }
}

#[test]
fn escape_long_name() {
    let long = "<".repeat(100_000);
    let mut c = SessionTreeController::new();
    let started = Instant::now();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, &long)],
        },
    );
    let v = c.view();
    assert!(
        v.rows[0].name.chars().count() <= 121,
        "the row is truncated by the view"
    );
    assert!(v.rows[0].name.starts_with("<<<<"));
    assert!(started.elapsed().as_millis() < 500, "the UI does not hang");
}

#[test]
fn tree_500_sessions_builds_fast() {
    let sessions: Vec<_> = (1..=500)
        .map(|i| session(i, Some(1 + i % 3), &format!("Session {i}")))
        .collect();
    let folders = vec![
        folder(1, None, "A", None),
        folder(2, None, "B", None),
        folder(3, Some(2), "C", None),
    ];
    let mut c = SessionTreeController::new();
    let started = Instant::now();
    send(&mut c, TreeMsg::Snapshot { folders, sessions });
    for i in 0..50 {
        send(
            &mut c,
            TreeMsg::Frame(TreeFrame::Unread {
                session_id: 1 + i,
                unread: 1,
            }),
        );
    }
    assert_eq!(c.view().rows.len(), 503);
    assert!(
        started.elapsed().as_millis() < 1500,
        "500 sessions and 50 frames: {:?}",
        started.elapsed()
    );
}
