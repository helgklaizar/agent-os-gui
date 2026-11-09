//! Permission request lifecycle: creation, first-wins decision, expiry after 10 minutes, cancellation on stop.

use serde_json::{Value, json};

use crate::chat_msgs::{self, NewMsg};
use crate::chat_state::{REQUEST_TTL_SECS, Req};
use crate::perm_store::{self as store, Mode, PutError};
use crate::state::Shared;
use crate::tree;

pub struct NewReq {
    pub session: i64,
    pub action: String,
    pub target: Option<String>,
    pub observer: Option<Observer>,
    pub detail: Option<String>,
    pub args: Option<Value>,
    /// Action blocked immediately (watcher unavailable): the request is born `denied`.
    pub closed: bool,
}

pub struct Observer {
    pub kind: String,
    pub verdict: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    Deny,
    Expired,
    Cancelled,
}

impl Outcome {
    fn status(self) -> &'static str {
        match self {
            Outcome::Allow => "allowed",
            Outcome::Deny => "denied",
            Outcome::Expired => "expired",
            Outcome::Cancelled => "cancelled",
        }
    }

    fn log(self) -> (&'static str, &'static str) {
        match self {
            Outcome::Allow => ("ask-approved", "approved"),
            Outcome::Deny => ("ask-denied", "denied"),
            Outcome::Expired => ("deny", "expired"),
            Outcome::Cancelled => ("deny", "cancelled"),
        }
    }
}

#[derive(Debug)]
pub enum ResolveErr {
    NotFound,
    /// Already decided; carries the current status.
    Decided(String),
    Expired,
    Loosen,
    Db,
}

fn card_meta(req: &Req) -> Value {
    json!({ "kind": "permission_request", "request_id": req.id, "request": req.json() })
}

/// Creates a request: a log row without a decision, a watcher finding, a card message, frames.
pub fn open(shared: &Shared, n: NewReq) -> Option<Req> {
    let now = shared.clock.now_unix();
    let built = shared.db.write(|w| {
        let persona = tree::effective_persona_id(w, n.session)?;
        let id = store::log_action(
            w,
            now,
            "agent",
            (Some(n.session), persona),
            (&n.action, n.target.as_deref()),
            n.args.as_ref(),
            (None, None),
        )?;
        let mut finding = None;
        if let Some(o) = &n.observer {
            let sev = if n.closed { "med" } else { "high" };
            finding = Some(store::finding(
                w,
                now,
                &o.kind,
                sev,
                Some("actions_log"),
                Some(&id.to_string()),
                &o.verdict,
                n.detail.as_deref(),
            )?);
        }
        if n.closed {
            store::set_decision(w, id, "deny", "observer_unavailable")?;
        }
        let observer = n.observer.is_some();
        let mut req = Req {
            id,
            session_id: Some(n.session),
            persona_id: persona,
            action: n.action.clone(),
            target: n.target.clone(),
            source: if observer { "observer" } else { "policy" },
            level: if observer { "warn" } else { "ask" },
            kind: n.observer.as_ref().map(|o| o.kind.clone()),
            verdict: n.observer.as_ref().map(|o| o.verdict.clone()),
            status: if n.closed { "denied" } else { "pending" }.to_string(),
            detail: n.detail.clone(),
            decided_by: n.closed.then(|| "observer".to_string()),
            decided_at: n.closed.then_some(now),
            expires_at: now + REQUEST_TTL_SECS,
            created_at: now,
            message_id: None,
            finding_id: finding,
        };
        let meta = card_meta(&req);
        let msg = chat_msgs::insert(
            w,
            now,
            &NewMsg {
                session: n.session,
                client_id: None,
                role: "system",
                author: None,
                body: "",
                status: "done",
                meta: Some(&meta),
            },
        )?;
        req.message_id = Some(msg);
        Ok(req)
    });
    let req = built.ok()?;
    shared.chat.lock().requests.insert(req.id, req.clone());
    if let Some(id) = req.message_id
        && let Ok(Some(m)) = shared
            .db
            .reader()
            .map(|c| chat_msgs::get(&c, id).ok().flatten())
    {
        chat_msgs::emit_created(shared, &m);
    }
    chat_msgs::emit(
        shared,
        req.session_id.unwrap_or(0),
        "permission.request",
        json!({ "request": req.json() }),
    );
    Some(req)
}

/// "Always" rule: `(scope, scope_id, mode)`.
pub type Rule<'a> = (&'a str, i64, Mode);

/// Decision, expiry or cancellation. The first decision wins: the status is taken under a lock.
pub fn resolve(
    shared: &Shared,
    id: i64,
    outcome: Outcome,
    by: &str,
    rule: Option<Rule<'_>>,
) -> Result<Req, ResolveErr> {
    let now = shared.clock.now_unix();
    let mut guard = shared.chat.lock();
    let cur = guard
        .requests
        .get(&id)
        .cloned()
        .ok_or(ResolveErr::NotFound)?;
    if cur.status != "pending" {
        return Err(ResolveErr::Decided(cur.status));
    }
    if matches!(outcome, Outcome::Allow | Outcome::Deny) && now >= cur.expires_at {
        drop(guard);
        let _ = resolve(shared, id, Outcome::Expired, "system", None);
        return Err(ResolveErr::Expired);
    }
    let mut next = cur.clone();
    next.status = outcome.status().to_string();
    next.decided_by = Some(by.to_string());
    next.decided_at = Some(now);
    let (decision, result) = outcome.log();
    let done = shared.db.write(|w| {
        if let Some((scope, sid, mode)) = rule {
            match store::put_rule(w, now, (scope, sid, &cur.action), mode, by, true) {
                Ok(()) => {
                    let target = format!("{scope}/{sid}/{}={}", cur.action, mode.as_str());
                    store::log_action(
                        w,
                        now,
                        "user",
                        (cur.session_id, None),
                        ("config_write", Some(&target)),
                        None,
                        (Some("allow"), Some("always")),
                    )?;
                }
                Err(PutError::Loosen) => return Ok(Err(ResolveErr::Loosen)),
                Err(PutError::Db(e)) => return Err(e),
            }
        }
        store::set_decision(w, id, decision, result)?;
        if let Some(f) = cur.finding_id {
            store::set_finding_verdict(
                w,
                f,
                if outcome == Outcome::Allow {
                    "allowed"
                } else {
                    "blocked"
                },
            )?;
        }
        if let Some(m) = cur.message_id {
            chat_msgs::set_meta(w, m, &card_meta(&next))?;
        }
        Ok(Ok(()))
    });
    match done {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(ResolveErr::Db),
    }
    guard.requests.insert(id, next.clone());
    drop(guard);
    let session = next.session_id.unwrap_or(0);
    chat_msgs::emit(
        shared,
        session,
        "permission.resolved",
        json!({ "request_id": id, "status": next.status, "decided_by": by, "result": result }),
    );
    follow_up(shared, &next, outcome);
    Ok(next)
}

/// Short agent reply after the decision: done or rejected.
fn follow_up(shared: &Shared, req: &Req, outcome: Outcome) {
    let (Some(session), true) = (
        req.session_id,
        matches!(outcome, Outcome::Allow | Outcome::Deny),
    ) else {
        return;
    };
    let target = req.target.clone().unwrap_or_default();
    if req.action == "shell" {
        match outcome {
            Outcome::Allow => drop(crate::term_sim::run(shared, session, &target)),
            _ => drop(crate::term_sim::denied(shared, session, &target)),
        }
    }
    let text = match (outcome, req.source) {
        (Outcome::Allow, "observer") => format!("Command confirmed: {target}"),
        (Outcome::Allow, _) => format!("Done: {} {target}", req.action),
        (_, "observer") => format!("Command blocked: {target}"),
        _ => format!("Rejected: {} {target}", req.action),
    };
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
            body: &text,
            status: "done",
            meta: None,
        },
    );
}
