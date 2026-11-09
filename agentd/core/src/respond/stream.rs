//! One model reply: context -> provider -> deltas to the DB and `session.<id>` -> message close -> accounting.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

use super::prompt::{account_settings, context_budget_tokens, normalize, price_of, system_text};
use super::sink::Sink;
use super::{Inner, topic};
use crate::{
    ChatContext, ModelAccount, ModelRequest, ProviderError, ProviderEvent, ProviderKind, RateLimit,
    ToolCall, Usage, UsageRecord, pick_account,
};

/// What is known about the reply after the stream.
#[derive(Default)]
struct Outcome {
    status: &'static str,
    error: Option<ProviderError>,
    usage: Option<Usage>,
    rate: Option<RateLimit>,
    calls: Vec<ToolCall>,
    /// Start of the reply text (up to `OUTPUT_MAX` bytes), used as the task run output.
    text: String,
}

/// Outcome of one model reply.
pub(super) struct Round {
    pub status: &'static str,
    pub calls: Vec<ToolCall>,
    pub ctx: ChatContext,
    pub text: String,
}

struct Target {
    account: ModelAccount,
    model: String,
    kind: ProviderKind,
    doc: Value,
}

fn failure(code: &'static str) -> Outcome {
    Outcome {
        status: "error",
        error: Some(ProviderError::new(code, false)),
        ..Outcome::default()
    }
}

/// Account, model and settings; an error is a ready `error` outcome (`no_model`, `unsupported`) with no outside call.
async fn resolve(inner: &Inner, ctx: &crate::ChatContext) -> Result<Target, &'static str> {
    let id = pick_account(ctx.cfg_model, ctx.persona_model_acc).ok_or("no_model")?;
    let row = match inner.store.account(id).await {
        Ok(Some(r)) => r,
        _ => return Err("no_model"),
    };
    let doc = inner
        .settings
        .get("models")
        .await
        .map(|d| d.doc)
        .unwrap_or_default();
    let s = account_settings(&doc, id);
    let model = s.model.ok_or("no_model")?;
    let kind = ProviderKind::from_account_type(&row.kind).ok_or("unsupported")?;
    let secret_ref = row
        .secret_ref
        .clone()
        .or_else(|| Some(format!("provider.{}", row.kind)));
    let account = ModelAccount {
        id,
        kind,
        label: row.label,
        base_url: s.base_url,
        secret_ref,
    };
    Ok(Target {
        account,
        model,
        kind,
        doc,
    })
}

fn meta_of(o: &Outcome, t: Option<&Target>) -> Option<Value> {
    let mut meta = serde_json::Map::new();
    if let Some(e) = &o.error {
        meta.insert(
            "error".into(),
            json!({"code": e.code, "retryable": e.retryable}),
        );
    }
    if let (Some(u), Some(t)) = (o.usage, t) {
        let mut usage = json!({"account_id": t.account.id, "model": t.model,
            "tokens_in": u.tokens_in, "tokens_out": u.tokens_out});
        if let Some(p) = price_of(&t.doc, &t.model) {
            usage["cost"] = json!(p.cost(u));
            usage["currency"] = json!(p.currency);
        }
        meta.insert("usage".into(), usage);
    }
    (!meta.is_empty()).then_some(Value::Object(meta))
}

/// One model reply; `None` means the session is gone. Result: status, tool calls, session context.
pub(super) async fn one_round(
    inner: &Arc<Inner>,
    session_id: i64,
    user_msg_id: i64,
    stop: &Arc<Notify>,
) -> Option<Round> {
    let Ok(ctx) = inner.store.context(session_id).await else {
        return None; // the session was deleted while the message waited in the queue
    };
    let author = ctx
        .persona_name
        .as_ref()
        .map_or_else(|| "Agent".to_string(), |p| format!("Agent · {p}"));
    let Ok(created) = inner.store.create_agent(session_id, author).await else {
        return None;
    };
    let id = created["id"].as_i64()?;
    let tp = topic(session_id);
    inner
        .notifier
        .publish_event(&tp, json!({"kind": "message.created", "message": created}));
    inner
        .notifier
        .publish_event(&tp, json!({"kind": "typing", "on": true}));

    let target = resolve(inner, &ctx).await;
    let (outcome, final_target) = match target {
        Err(code) => (failure(code), None),
        Ok(t) => {
            let o = run_model(inner, &ctx, &t, session_id, user_msg_id, id, stop).await;
            (o, Some(t))
        }
    };
    let status = outcome.status;
    let meta = meta_of(&outcome, final_target.as_ref());
    let Ok(final_len) = inner.store.finish(id, status, meta).await else {
        return None; // the message vanished together with the session
    };
    inner.notifier.publish_event(
        &tp,
        json!({"kind": "message.done", "message_id": id, "status": status, "final_len": final_len}),
    );
    if let (Some(e), Some(t)) = (&outcome.error, &final_target) {
        super::events::provider_failed(inner, session_id, t.kind, e.code).await;
    }
    inner
        .notifier
        .publish_event(&tp, json!({"kind": "typing", "on": false}));
    if let (Some(q), Some(t)) = (&inner.quotas, &final_target) {
        record_quota(inner, q, t, &outcome, session_id).await;
    }
    Some(Round {
        status,
        calls: outcome.calls,
        ctx,
        text: outcome.text,
    })
}

async fn record_quota(
    inner: &Inner,
    q: &Arc<dyn crate::QuotaStore>,
    t: &Target,
    o: &Outcome,
    session_id: i64,
) {
    if o.usage.is_none() && o.rate.is_none() {
        return;
    }
    let rec = UsageRecord {
        account_id: t.account.id,
        local: t.kind.is_local(),
        session_id,
        rate: o.rate,
        now: now_unix(),
    };
    if let Some(quota) = q.record(rec).await {
        inner.notifier.publish("bar.quota", quota);
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Tool catalog for the model: with an action dispatcher, built-in and MCP tools; without one, the model gets no tools
/// (there would be nothing to run them with anyway).
async fn catalog(inner: &Inner) -> Vec<crate::ToolSpec> {
    let Some(actions) = &inner.actions else {
        return Vec::new();
    };
    let mut tools = crate::builtin_tools();
    if let Some(m) = &actions.mcp {
        tools.extend(m.catalog().await);
    }
    tools
}

async fn run_model(
    inner: &Inner,
    ctx: &crate::ChatContext,
    t: &Target,
    session_id: i64,
    user_msg_id: i64,
    id: i64,
    stop: &Notify,
) -> Outcome {
    let Some(provider) = inner.providers.get(t.kind) else {
        return failure("unsupported");
    };
    let budget_bytes = context_budget_tokens(&t.doc).saturating_mul(4);
    let Ok(items) = inner
        .store
        .history(session_id, user_msg_id + 1, ctx.ctx_parent, budget_bytes)
        .await
    else {
        return failure("internal");
    };
    let req = ModelRequest {
        account: t.account.clone(),
        model: t.model.clone(),
        system: system_text(ctx),
        history: normalize(items),
        max_tokens: inner.cfg.max_tokens,
        tools: catalog(inner).await,
    };
    let (tx, mut rx) = mpsc::channel::<ProviderEvent>(64);
    let mut provider_fut = Box::pin(provider.stream(req, tx));
    let mut provider_done = false;
    let deadline = sleep(inner.cfg.total_timeout);
    tokio::pin!(deadline);
    let mut ticker = interval_at(
        Instant::now() + inner.cfg.flush_every,
        inner.cfg.flush_every,
    );
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sink = Sink {
        inner,
        session_id,
        message_id: id,
        buf: String::new(),
        len: 0,
        alive: true,
    };
    let mut out = Outcome {
        status: "done",
        ..Outcome::default()
    };
    loop {
        tokio::select! {
            biased;
            () = stop.notified() => { out.status = "stopped"; break; }
            () = &mut deadline => {
                out.status = "error";
                out.error = Some(ProviderError::new("timeout", true));
                break;
            }
            () = &mut provider_fut, if !provider_done => { provider_done = true; }
            ev = rx.recv() => match ev {
                None => break,
                Some(ProviderEvent::Delta(text)) => {
                    super::origin::push_capped(&mut out.text, &text);
                    sink.buf.push_str(&text);
                    if sink.buf.len() >= inner.cfg.flush_bytes { sink.flush().await; }
                }
                Some(ProviderEvent::Usage(u)) => out.usage = Some(u),
                Some(ProviderEvent::RateLimit(r)) => out.rate = Some(r),
                Some(ProviderEvent::Error(e)) => { out.status = "error"; out.error = Some(e); }
                Some(ProviderEvent::ToolCall(c)) => out.calls.push(c),
            },
            _ = ticker.tick() => sink.flush().await,
        }
        if !sink.alive {
            break;
        }
    }
    drop(provider_fut);
    sink.flush().await;
    out
}
