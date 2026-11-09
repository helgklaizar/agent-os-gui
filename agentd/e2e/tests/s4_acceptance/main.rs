//! A4-11: S4 acceptance suite against the real `agentd` binary with the `scripted` provider (rows of the
//! "Integration" table). `weaken_attempt_logged`, `ask_expires_after_10_min`,
//! `observer_unavailable_dangerous_fail_closed`, `observer_dangerous_cmd_flow` and `stream_resubscribe_no_dup_by_offset`
//! live next to the code they cover: `permissions_api_session.rs`, `permission_decision.rs`, `dispatcher_observer.rs`,
//! `chat_resume.rs`.

#[path = "../common/mod.rs"]
mod common;

mod ask;
mod chat;
