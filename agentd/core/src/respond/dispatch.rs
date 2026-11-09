//! Agent action dispatcher: tool call -> `PolicyGate` -> (card and wait for a decision) -> executor ->
//! result for the model. Any kind of refusal goes back to the model as a tool error with a clear code.

use serde_json::{Value, json};
use tokio::sync::Notify;

use super::Inner;
use crate::{
    Action, ActionCall, Actor, ChatContext, ExecError, Gate, GateInput, Outcome, Subject, ToolCall,
    action_for_tool, target_of,
};

/// Call outcome for the model and for the `role = tool` message.
pub(super) struct Report {
    pub ok: bool,
    pub code: String,
    pub action: Option<Action>,
    pub call_id: String,
    /// What the executor returned (`browse` result: `truncated`, `output_bytes`, `output`).
    pub detail: Option<Value>,
    /// Untrusted text from an external source for the model: already sanitized by the observer and truncated.
    pub untrusted_output: Option<String>,
    /// Where the untrusted text came from (`origin` in the tool result).
    pub origin: String,
}

/// Untrusted text source after `browse`.
pub(super) const BROWSE_ORIGIN: &str = "from the page";
/// Untrusted text source after `shell` (command output).
const SHELL_ORIGIN: &str = "from command output";
/// How many bytes of untrusted text the model sees.
const MODEL_OUTPUT_BYTES: usize = 8 * 1024;
/// Replaces text in which the observer found injection markers.
const ISOLATED: &str = "[text isolated by the observer: injection markers]";

impl Report {
    fn new(call: &ToolCall, action: Option<Action>, ok: bool, code: &str) -> Report {
        Report {
            ok,
            code: code.to_string(),
            action,
            call_id: call.id.clone(),
            detail: None,
            untrusted_output: None,
            origin: BROWSE_ORIGIN.to_string(),
        }
    }

    /// Result text for the model: a single line so the message does not break when history is joined.
    pub fn body(&self) -> String {
        let mut doc =
            json!({"action": self.action.map(Action::as_str), "code": self.code, "ok": self.ok});
        if let (Some(out), Value::Object(m)) = (&self.untrusted_output, &mut doc) {
            // External source text is marked: the model must not take it for an instruction.
            m.insert("untrusted".into(), true.into());
            m.insert("origin".into(), self.origin.clone().into());
            m.insert("output".into(), out.clone().into());
        }
        format!("[tool_result {doc}]")
    }

    pub fn meta(&self) -> Value {
        let mut meta = json!({"kind": "tool_result", "ok": self.ok, "code": self.code,
               "action": self.action.map(Action::as_str), "call_id": self.call_id});
        // Browser output sizes are visible to the client; the output itself is not put into `meta`.
        if let (Some(d), Value::Object(m)) = (&self.detail, &mut meta) {
            for k in ["truncated", "output_bytes"] {
                if let Some(v) = d.get(k).filter(|v| !v.is_null()) {
                    m.insert(k.into(), v.clone());
                }
            }
        }
        meta
    }
}

/// Tool error code for the model from a request refusal.
fn denial_code(reason: &str) -> &'static str {
    match reason {
        "expired" => "expired",
        "session_deleted" => "session_deleted",
        "cancelled" => "stopped",
        "closed" => "closed",
        _ => "denied",
    }
}

pub(super) async fn dispatch(
    inner: &Inner,
    ctx: &ChatContext,
    session_id: i64,
    call: &ToolCall,
    untrusted: Option<&str>,
    stop: &Notify,
    actor: Actor,
) -> Report {
    let Some(actions) = &inner.actions else {
        return Report::new(call, None, false, "actions_unavailable");
    };
    let mcp_tool = match &actions.mcp {
        Some(m) => m.resolve(&call.name).await,
        None => None,
    };
    let action = match &mcp_tool {
        Some(t) => t.action,
        None => match action_for_tool(&call.name) {
            Some(a) => a,
            None => return Report::new(call, None, false, "unknown_tool"),
        },
    };
    let fail = |code: &str| Report::new(call, Some(action), false, code);
    let target = mcp_tool
        .as_ref()
        .map_or_else(|| target_of(&call.args), crate::McpTool::target);
    let input = GateInput {
        subject: Subject {
            actor,
            session_id: Some(session_id),
            persona_id: ctx.persona_id,
        },
        action,
        target: Some(target.clone()),
        args: Some(call.args.clone()),
        untrusted: untrusted.map(str::to_string),
        grant_token: None,
    };
    let Ok(verdict) = actions.gate.check(input).await else {
        return fail("internal");
    };
    let action_id = match verdict {
        Gate::Deny { reason, .. } => {
            return fail(if reason == "observer_blocked" {
                "blocked_by_observer"
            } else {
                "permission_denied"
            });
        }
        Gate::Allow { action_id } => action_id,
        Gate::Ask(req) => {
            let requests = actions.gate.requests();
            let outcome = tokio::select! {
                o = requests.wait(req.id) => o,
                () = stop.notified() => {
                    requests.cancel(req.id).await;
                    return fail("stopped");
                }
            };
            match outcome {
                Outcome::Allowed => req.id,
                Outcome::Denied(reason) => return fail(denial_code(reason)),
            }
        }
    };
    let exec = ActionCall {
        action,
        target,
        args: call.args.clone(),
        session_id,
    };
    let res = tokio::select! {
        r = execute(actions, mcp_tool.as_ref(), &exec) => r,
        () = stop.notified() => return fail("stopped"),
    };
    let tool_failed = res
        .as_ref()
        .is_ok_and(|v| v.get("is_error") == Some(&Value::Bool(true)));
    let (ok, code) = match &res {
        Ok(_) if tool_failed => (false, "tool_error"),
        Ok(_) => (true, "ok"),
        Err(e) => (false, e.code),
    };
    // The execution outcome goes to the log: it is truthful even when there is no executor.
    if actions
        .gate
        .audit()
        .store()
        .set_result(action_id, code.to_string())
        .await
        .is_err()
    {
        tracing::warn!(
            scope = "actions",
            event = "result_not_logged",
            "action outcome not recorded"
        );
    }
    let mut report = Report::new(call, Some(action), ok, code);
    report.detail = res.ok();
    if (action == Action::Browse || action == Action::Shell || mcp_tool.is_some())
        && (ok || tool_failed)
    {
        report.untrusted_output =
            Some(untrusted_text(actions, action_id, report.detail.as_ref()).await);
    }
    if action == Action::Shell {
        report.origin = SHELL_ORIGIN.to_string();
    }
    if let Some(t) = &mcp_tool {
        report.origin = format!("from MCP server {}", t.server);
    }
    report
}

/// Execution after the gate decision: an MCP tool goes through the client, everything else through the action executor.
async fn execute(
    actions: &super::Actions,
    mcp_tool: Option<&crate::McpTool>,
    call: &ActionCall,
) -> Result<Value, ExecError> {
    let (Some(tool), Some(mcp)) = (mcp_tool, &actions.mcp) else {
        return actions.executors.get(call.action).execute(call).await;
    };
    let r = mcp.call(tool, &call.args).await?;
    // Server error text reaches the model by the same untrusted path as normal output (the `is_error` flag is for the outcome code).
    Ok(json!({"output": r.text, "is_error": r.is_error}))
}

/// Browser output for the model: the observer checks for injection markers (an `injection` finding and text replacement),
/// then truncation to `MODEL_OUTPUT_BYTES`.
async fn untrusted_text(
    actions: &super::Actions,
    action_id: i64,
    detail: Option<&Value>,
) -> String {
    let text = detail
        .and_then(|d| d.get("output"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(hit) = actions.gate.observer().check_text(text) {
        let f = crate::Finding {
            kind: hit.kind,
            severity: hit.severity,
            ref_table: Some("actions_log"),
            ref_id: Some(action_id.to_string()),
            verdict: hit.verdict,
            detail: Some(format!("rule={}", hit.rule)),
        };
        if actions.gate.audit().finding(f).await.is_err() {
            tracing::warn!(
                scope = "observer",
                event = "finding_lost",
                "finding not recorded"
            );
        }
        return ISOLATED.to_string();
    }
    let mut end = text.len().min(MODEL_OUTPUT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
