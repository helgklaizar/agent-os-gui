//! Agent terminal in the mock (api.md section 4.4): the "core" runs the agent command in its own pty and sends the frames
//! `terminal.command`, `terminal.status`, `terminal.output`; there is no real process, the output is synthetic.
//! The GUI only displays these frames and does not return a result.

use serde_json::json;

use crate::chat_msgs;
use crate::state::Shared;

const CWD: &str = "~";

fn command_frame(shared: &Shared, session: i64, id: &str, command: &str) {
    chat_msgs::emit(
        shared,
        session,
        "terminal.command",
        json!({ "command": command, "cwd": CWD, "request_id": id }),
    );
}

fn status(shared: &Shared, session: i64, id: &str, status: &str, extra: serde_json::Value) {
    let mut f = json!({ "request_id": id, "status": status });
    if let (Some(o), Some(e)) = (f.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    chat_msgs::emit(shared, session, "terminal.status", f);
}

/// Command executed: `started`, output in two chunks, `exited` with code 0.
pub fn run(shared: &Shared, session: i64, command: &str) -> String {
    let id = shared.tools.open(session, "terminal");
    command_frame(shared, session, &id, command);
    status(shared, session, &id, "started", json!({}));
    for (seq, data) in [format!("$ {command}\n"), "done\n".to_string()]
        .into_iter()
        .enumerate()
    {
        chat_msgs::emit(
            shared,
            session,
            "terminal.output",
            json!({ "request_id": id, "seq": seq, "stream": "stdout", "data": data }),
        );
    }
    status(
        shared,
        session,
        &id,
        "exited",
        json!({ "exit_code": 0, "duration_ms": 12 }),
    );
    id
}

/// Policy or watcher refusal: the command was not started.
pub fn denied(shared: &Shared, session: i64, command: &str) -> String {
    let id = shared.tools.open(session, "terminal");
    command_frame(shared, session, &id, command);
    status(shared, session, &id, "denied", json!({}));
    id
}
