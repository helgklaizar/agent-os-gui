//! Live connection: subscriptions, snapshots and `seq`, `ping`/`pong`, outbound queue (api.md sections 4, 7).

use std::collections::{BTreeSet, HashMap};

use tokio::time::{Instant, MissedTickBehavior};

use crate::api::{ClientOp, FrameKind, ServerMessage, parse_server_message};
use crate::chat::SendOutcome;
use crate::core::{Command, CoreEvent};
use crate::error::ClientError;
use crate::queue::Target;
use crate::state::OfflineReason;
use crate::supervisor::{Core, Flow};
use crate::ws::{WsConn, WsIn};

pub(crate) enum End {
    Shutdown,
    Lost,
    Fatal(OfflineReason),
}

enum Flush {
    Next,
    Lost,
    Fatal(OfflineReason),
}

/// WS close codes (api.md sections 6, 7): 4401, 4403, 4426 must not be retried.
fn close_end(code: Option<u16>) -> End {
    match code {
        Some(4401) => End::Fatal(OfflineReason::TokenRejected),
        Some(4403) => End::Fatal(OfflineReason::Forbidden),
        Some(4426) => End::Fatal(OfflineReason::ApiVersionUnsupported),
        _ => End::Lost,
    }
}

struct Live {
    ws: WsConn,
    /// Topics whose snapshot has not arrived yet: while any exist the queue waits (at most `sync_timeout`).
    pending: BTreeSet<String>,
    last_seq: HashMap<String, u64>,
    missed: u32,
}

impl Live {
    async fn op(&mut self, op: ClientOp) -> bool {
        self.ws.send_op(&op).await.is_ok()
    }

    async fn subscribe(&mut self, topic: &str) -> bool {
        self.pending.insert(topic.to_string());
        self.last_seq.remove(topic);
        self.op(ClientOp::Subscribe(topic.to_string())).await
    }

    /// Server text frame; `false` means the socket write failed.
    async fn on_text(&mut self, core: &Core, text: &str) -> bool {
        let Some(msg) = parse_server_message(text) else {
            return true;
        };
        let ServerMessage::Frame(frame) = msg else {
            self.missed = 0;
            return true;
        };
        if !core.subs.contains(&frame.topic) {
            return true;
        }
        let last = self.last_seq.get(&frame.topic).copied();
        match (frame.kind, last) {
            (FrameKind::Snapshot, _) => {
                self.pending.remove(&frame.topic);
                self.last_seq.insert(frame.topic.clone(), frame.seq);
            }
            (_, Some(last)) if frame.seq == last + 1 => {
                self.last_seq.insert(frame.topic.clone(), frame.seq);
            }
            (_, Some(last)) if frame.seq > last + 1 => {
                // Gap: the frame is not delivered, the subscription is redone and a fresh snapshot arrives (api.md section 4.2).
                let topic = frame.topic.clone();
                return self.op(ClientOp::Unsubscribe(topic.clone())).await
                    && self.subscribe(&topic).await;
            }
            _ => return true,
        }
        core.emit(CoreEvent::Frame(frame));
        true
    }
}

/// Sends the head of the queue: `2xx` delivered, `4xx` error without retry, network and `5xx` keep the item (api.md section 7).
async fn flush_one(core: &mut Core) -> Flush {
    let Some(item) = core.queue.front().cloned() else {
        return Flush::Next;
    };
    let sent = match &item.target {
        Target::Echo => core
            .http
            .echo(&item.client_id, &item.text)
            .await
            .map(|r| r.duplicate),
        Target::Message {
            session_id,
            attachment_ids,
        } => core
            .http
            .send_message(*session_id, &item.client_id, &item.text, attachment_ids)
            .await
            .map(|o| matches!(o, SendOutcome::Duplicate(_))),
    };
    match sent {
        Ok(duplicate) => {
            core.queue.pop_front();
            core.emit(CoreEvent::Delivered {
                client_id: item.client_id,
                duplicate,
            });
            Flush::Next
        }
        Err(ClientError::Api(e)) if e.status == 401 => Flush::Fatal(OfflineReason::TokenRejected),
        Err(ClientError::Api(e)) if e.status == 429 || e.status >= 500 => Flush::Lost,
        Err(ClientError::Api(e)) => {
            core.queue.pop_front();
            core.emit(CoreEvent::Failed {
                client_id: item.client_id,
                error: *e,
            });
            Flush::Next
        }
        Err(_) => Flush::Lost,
    }
}

pub(crate) async fn run(core: &mut Core, ws: WsConn) -> End {
    let mut live = Live {
        ws,
        pending: BTreeSet::new(),
        last_seq: HashMap::new(),
        missed: 0,
    };
    for topic in core.subs.clone() {
        if !live.subscribe(&topic).await {
            return End::Lost;
        }
    }
    let sync_deadline = Instant::now() + core.cfg.sync_timeout;
    let mut ping = tokio::time::interval(core.cfg.ping_interval);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.reset();
    loop {
        if core.stopped() {
            return End::Shutdown;
        }
        let waiting = !live.pending.is_empty() && Instant::now() < sync_deadline;
        if !core.queue.is_empty() && !waiting {
            match flush_one(core).await {
                Flush::Next => continue,
                Flush::Lost => return End::Lost,
                Flush::Fatal(r) => return End::Fatal(r),
            }
        }
        tokio::select! {
            msg = live.ws.next() => match msg {
                Some(Ok(WsIn::Text(t))) => {
                    if !live.on_text(core, &t).await {
                        return End::Lost;
                    }
                }
                Some(Ok(WsIn::Close(code))) => return close_end(code),
                Some(Err(_)) | None => return End::Lost,
            },
            cmd = core.cmds.recv() => {
                let Ok(cmd) = cmd else { return End::Shutdown };
                if let Some(end) = on_command(core, &mut live, cmd).await {
                    return end;
                }
            }
            _ = ping.tick() => {
                if live.missed >= 2 || !live.op(ClientOp::Ping).await {
                    return End::Lost;
                }
                live.missed += 1;
            }
            () = tokio::time::sleep_until(sync_deadline), if waiting && !core.queue.is_empty() => {}
        }
    }
}

/// `Some` means the session must end.
async fn on_command(core: &mut Core, live: &mut Live, cmd: Command) -> Option<End> {
    let wrote = match cmd {
        Command::Subscribe(t) => !core.subs.insert(t.clone()) || live.subscribe(&t).await,
        Command::Unsubscribe(t) => {
            live.pending.remove(&t);
            live.last_seq.remove(&t);
            !core.subs.remove(&t) || live.op(ClientOp::Unsubscribe(t)).await
        }
        other => match core.apply(other) {
            Flow::Go => true,
            Flow::Stop => {
                live.ws.close().await;
                return Some(End::Shutdown);
            }
        },
    };
    (!wrote).then_some(End::Lost)
}
