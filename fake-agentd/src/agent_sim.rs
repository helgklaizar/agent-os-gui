//! Agent simulation ("simulation mode"): the reply arrives after `Config::reply_delay` and streams in chunks.
//!
//! Behaviour is chosen by the start of the user message text:
//! - plain text - a reply "Accepted: <text>" in chunks of 8 characters;
//! - `/ask <action> <target> [JSON args]` - a permission request by the effective mode: `allow` runs without a card,
//!   `deny` refuses, `ask` creates a card (when the watcher is unavailable a dangerous action always asks);
//! - `/danger <command>` - a watcher card (`dangerous_cmd`); when the watcher is unavailable the action is blocked at once;
//! - `/html` - a reply with HTML and markup; `/long` - 100,000 characters; `/error` - the stream ends with status `error`;
//! - `/slow` - a long stream (60 words of 100 ms) for stopping; `/drop <text>` - after the second chunk drops the WS,
//!   waits for `restore`, then repeats the last delta and finishes the reply.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::chat_msgs::{self, NewMsg};
use crate::perm_flow::{self, NewReq, Observer};
use crate::perm_store::{self as store, Mode};
use crate::perm_sweep;
use crate::state::Shared;

const UNAVAILABLE: &str = "watcher unavailable: action blocked until confirmed (fail-closed)";

pub fn on_user_message(shared: &Arc<Shared>, session: i64, text: String) {
    let flag = shared.chat.begin_stream(session);
    let shared = shared.clone();
    tokio::spawn(async move {
        run(&shared, session, &text, &flag).await;
        shared.chat.end_stream(session, &flag);
    });
}

async fn run(shared: &Arc<Shared>, session: i64, text: &str, flag: &Arc<AtomicBool>) {
    let delay = shared.chat.reply_delay();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    if flag.load(Ordering::SeqCst) {
        return;
    }
    let (cmd, rest) = text.split_once(' ').unwrap_or((text, ""));
    let rest = rest.trim();
    let plan = match cmd {
        "/ask" => return ask(shared, session, rest),
        "/danger" => return danger(shared, session, rest),
        "/html" => Plan::chunks(vec![
            "<script>alert(1)</script> ".into(),
            "<img src=x onerror=alert(1)> **bold** [x](javascript:alert(1))".into(),
        ]),
        "/long" => Plan::long(),
        "/error" => Plan {
            end: "error",
            ..Plan::chunks(vec!["Starting processing… ".into(), "failure".into()])
        },
        "/slow" => Plan {
            pause: Duration::from_millis(100),
            ..Plan::chunks((0..60).map(|i| format!("word{i} ")).collect())
        },
        "/drop" => Plan {
            drop_after: Some(2),
            ..Plan::chunks(split8(&format!("Accepted: {rest}")))
        },
        _ => Plan::chunks(split8(&format!("Accepted: {text}"))),
    };
    stream(shared, session, flag, plan).await;
}

fn split8(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(8).map(|c| c.iter().collect()).collect()
}

struct Plan {
    chunks: Vec<String>,
    pause: Duration,
    end: &'static str,
    drop_after: Option<usize>,
}

impl Plan {
    fn chunks(chunks: Vec<String>) -> Plan {
        Plan {
            chunks,
            pause: Duration::from_millis(10),
            end: "done",
            drop_after: None,
        }
    }

    fn long() -> Plan {
        let word: Vec<char> = "abcde ".chars().collect();
        let all: Vec<char> = word.iter().cycle().take(100_000).copied().collect();
        let chunks = all.chunks(5_000).map(|c| c.iter().collect()).collect();
        Plan {
            pause: Duration::from_millis(1),
            ..Plan::chunks(chunks)
        }
    }
}

fn stopped(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

async fn stream(shared: &Arc<Shared>, session: i64, flag: &AtomicBool, plan: Plan) {
    let author = shared
        .db
        .reader()
        .map(|c| chat_msgs::author(&c, session))
        .unwrap_or_default();
    chat_msgs::emit(shared, session, "typing", json!({ "on": true }));
    let msg = chat_msgs::post(
        shared,
        &NewMsg {
            session,
            client_id: None,
            role: "agent",
            author: Some(&author),
            body: "",
            status: "streaming",
            meta: None,
        },
    );
    let Some(id) = msg.and_then(|m| m["id"].as_i64()) else {
        return;
    };
    let mut sent = 0usize;
    let mut status = plan.end;
    for (i, chunk) in plan.chunks.iter().enumerate() {
        tokio::time::sleep(plan.pause).await;
        if stopped(flag) {
            status = "stopped";
            break;
        }
        if shared
            .db
            .write(|w| chat_msgs::append(w, id, chunk))
            .is_err()
        {
            status = "error";
            break;
        }
        let delta = json!({ "message_id": id, "offset": sent, "text": chunk });
        chat_msgs::emit(shared, session, "message.delta", delta.clone());
        sent += chunk.len();
        if plan.drop_after == Some(i + 1) {
            shared.hub.drop_ws();
            for _ in 0..3000 {
                if !shared.hub.is_dropped() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            // let the client reconnect and take a snapshot, then repeat the already sent delta
            tokio::time::sleep(Duration::from_millis(300)).await;
            chat_msgs::emit(shared, session, "message.delta", delta);
        }
    }
    let _ = shared.db.write(|w| chat_msgs::set_status(w, id, status));
    chat_msgs::emit(
        shared,
        session,
        "message.done",
        json!({ "message_id": id, "status": status, "final_len": sent }),
    );
    chat_msgs::emit(shared, session, "typing", json!({ "on": false }));
    crate::events_sim::agent_finished(shared, session, id, status);
}

fn say(shared: &Shared, session: i64, text: &str) {
    let author = shared
        .db
        .reader()
        .map(|c| chat_msgs::author(&c, session))
        .unwrap_or_default();
    chat_msgs::post(
        shared,
        &NewMsg {
            session,
            client_id: None,
            role: "agent",
            author: Some(&author),
            body: text,
            status: "done",
            meta: None,
        },
    );
}

/// `<action> <target> [JSON]`.
fn parse_ask(rest: &str) -> (String, String, Option<Value>) {
    let (action, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    let (target, args) = match tail.find('{') {
        Some(i) => (tail[..i].trim(), serde_json::from_str(&tail[i..]).ok()),
        None => (tail.trim(), None),
    };
    (action.to_string(), target.to_string(), args)
}

fn ask(shared: &Arc<Shared>, session: i64, rest: &str) {
    let (action, target, args) = parse_ask(rest);
    if !store::ACTIONS.contains(&action.as_str()) {
        return say(shared, session, "Unknown action");
    }
    let Ok(conn) = shared.db.reader() else { return };
    let mut mode = store::effective(&conn, Some(session), &action).unwrap_or(Mode::Ask);
    let persona = crate::tree::effective_persona_id(&conn, session)
        .ok()
        .flatten();
    let mut detail = None;
    if !shared.chat.observer_up() && store::dangerous(&action) && mode == Mode::Allow {
        mode = Mode::Ask;
    }
    if !shared.chat.observer_up() && store::dangerous(&action) {
        detail = Some(UNAVAILABLE.to_string());
    }
    let now = shared.clock.now_unix();
    let log = |decision: &str, result: &str| {
        let _ = shared.db.write(|w| {
            store::log_action(
                w,
                now,
                "agent",
                (Some(session), persona),
                (&action, Some(&target)),
                args.as_ref(),
                (Some(decision), Some(result)),
            )
        });
    };
    match mode {
        Mode::Allow => {
            log("allow", "ok");
            if action == "shell" {
                crate::term_sim::run(shared, session, &target);
            }
            say(shared, session, &format!("Done: {action} {target}"));
        }
        Mode::Deny => {
            log("deny", "denied_by_policy");
            if action == "shell" {
                crate::term_sim::denied(shared, session, &target);
            }
            say(
                shared,
                session,
                &format!("Denied by rule: {action} {target}"),
            );
        }
        Mode::Ask => {
            say(
                shared,
                session,
                &format!("Confirmation required: {action} {target}"),
            );
            let target = (!target.is_empty()).then_some(target);
            perm_flow::open(
                shared,
                NewReq {
                    session,
                    action,
                    target,
                    observer: None,
                    detail,
                    args,
                    closed: false,
                },
            );
        }
    }
}

fn danger(shared: &Arc<Shared>, session: i64, cmd: &str) {
    let up = shared.chat.observer_up();
    let (verdict, detail) = if up {
        (
            "asked",
            "dangerous command: confirmation required".to_string(),
        )
    } else {
        ("blocked", UNAVAILABLE.to_string())
    };
    let req = NewReq {
        session,
        action: "shell".into(),
        target: Some(cmd.to_string()),
        observer: Some(Observer {
            kind: "dangerous_cmd".into(),
            verdict: verdict.into(),
        }),
        detail: Some(detail),
        args: None,
        closed: !up,
    };
    perm_flow::open(shared, req);
    if !up {
        crate::term_sim::denied(shared, session, cmd);
    }
    say(
        shared,
        session,
        &format!("Command stopped pending decision: {cmd}"),
    );
}

/// `POST /sessions/{id}/stop`: the stream gets `stopped`, open requests get `cancelled`.
pub fn stop(shared: &Shared, session: i64) {
    shared.chat.stop_stream(session);
    shared.tools.close_session(session);
    perm_sweep::cancel_open(shared, session);
}
