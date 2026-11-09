//! Bulk permission request transitions: expiry on the virtual clock and cancellation on stop.

use crate::perm_flow::{Outcome, resolve};
use crate::state::Shared;

/// Overdue requests become `expired` (`deny`, action not performed).
pub fn sweep_expired(shared: &Shared) {
    let now = shared.clock.now_unix();
    let due: Vec<i64> = shared
        .chat
        .lock()
        .requests
        .values()
        .filter(|r| r.status == "pending" && now >= r.expires_at)
        .map(|r| r.id)
        .collect();
    for id in due {
        let _ = resolve(shared, id, Outcome::Expired, "system", None);
    }
}

/// `stop`: the session's open requests get `cancelled`.
pub fn cancel_open(shared: &Shared, session: i64) {
    let open: Vec<i64> = shared
        .chat
        .lock()
        .requests
        .values()
        .filter(|r| r.status == "pending" && r.session_id == Some(session))
        .map(|r| r.id)
        .collect();
    for id in open {
        let _ = resolve(shared, id, Outcome::Cancelled, "system", None);
    }
}
