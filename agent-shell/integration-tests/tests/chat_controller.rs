//! `ChatController` against `fake-agentd` over a real socket: `session.<id>` frames -> controller -> commands -> core.
mod common;

use std::time::{Duration, Instant};

use common::{WAIT, client, http, start};
use controllers::chat_view::{ButtonKind, CardState, ChatItem, ChatView, ObserverCard};
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::permissions::PermDecision;
use controllers::{Controller, CoreCommand, Effect};
use core_client::{CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use ui::chat_exec::{DecisionOutcome, decide_outcome, upload_all};
use ui::chat_feed::{apply_personas, route, topic};

const SESSION: i64 = 1;

struct Rig {
    d: FakeAgentd,
    c: CoreClient,
    http: HttpClient,
    chat: ChatController,
    rx: async_channel::Receiver<CoreEvent>,
}

async fn rig() -> Rig {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    let http = http(&d, Some(d.admin_token()));
    let mut chat = ChatController::new();
    assert!(c.subscribe(&topic(SESSION)).is_ok());
    let _ = chat.update(ChatMsg::Select(Some(SESSION)), Instant::now());
    let list = http.list_personas().await.unwrap_or_default();
    let _ = apply_personas(&mut chat, &list, Instant::now());
    Rig {
        d,
        c,
        http,
        chat,
        rx,
    }
}

impl Rig {
    async fn effects(&mut self, effects: Vec<Effect>) {
        let mut queue = effects;
        while let Some(e) = queue.pop() {
            let Effect::Command(cmd) = e else { continue };
            match cmd {
                CoreCommand::SendMessage {
                    session_id,
                    client_id,
                    body_md,
                    attachments,
                } => {
                    let ids = upload_all(&self.http, session_id, &attachments)
                        .await
                        .unwrap_or_default();
                    assert!(
                        self.c
                            .enqueue_message(session_id, &client_id, &body_md, &ids)
                            .is_ok()
                    );
                }
                CoreCommand::DecidePermission {
                    request_id,
                    decision,
                    rule,
                } => {
                    let msg = match decide_outcome(&self.http, request_id, decision, rule).await {
                        DecisionOutcome::Done => None,
                        DecisionOutcome::Rejected(status) => {
                            Some(ChatMsg::DecisionRejected { request_id, status })
                        }
                        DecisionOutcome::Failed => Some(ChatMsg::DecisionFailed { request_id }),
                    };
                    if let Some(m) = msg {
                        queue.extend(self.chat.update(m, Instant::now()));
                    }
                }
                CoreCommand::ResyncSession { session_id } => {
                    let t = topic(session_id);
                    assert!(self.c.unsubscribe(&t).is_ok() && self.c.subscribe(&t).is_ok());
                }
                other => {
                    let r = ui::tree_exec::execute(&self.http, &other).await;
                    assert!(r.as_ref().is_none_or(|r| r.is_ok()), "{other:?}: {r:?}");
                }
            }
        }
    }

    async fn say(&mut self, text: &str) {
        let _ = self
            .chat
            .update(ChatMsg::DraftChanged(text.to_string()), Instant::now());
        let fx = self.chat.update(
            ChatMsg::Send {
                client_id: uuid::Uuid::now_v7().to_string(),
            },
            Instant::now(),
        );
        self.effects(fx).await;
    }

    async fn click(&mut self, request_id: i64, button: ButtonKind) {
        let fx = self
            .chat
            .update(ChatMsg::CardButton { request_id, button }, Instant::now());
        self.effects(fx).await;
    }

    async fn pump(&mut self, what: &str, ok: impl Fn(&ChatView) -> bool) -> ChatView {
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            if ok(&self.chat.view()) {
                return self.chat.view();
            }
            if let Ok(Ok(ev)) =
                tokio::time::timeout(Duration::from_millis(100), self.rx.recv()).await
            {
                let routed = route(&mut self.chat, &ev, Instant::now());
                self.effects(routed.effects).await;
            }
        }
        panic!(
            "timed out waiting for: {what}; view: {:?}",
            self.chat.view().items
        );
    }
}

fn ask_card(v: &ChatView) -> Option<(i64, CardState)> {
    v.items.iter().find_map(|i| match i {
        ChatItem::Ask(c) => Some((c.request_id, c.state)),
        _ => None,
    })
}

fn observer(v: &ChatView) -> Option<ObserverCard> {
    v.items.iter().find_map(|i| match i {
        ChatItem::Observer(c) => Some(c.clone()),
        _ => None,
    })
}

fn agent_text(v: &ChatView) -> Option<String> {
    v.items.iter().find_map(|i| match i {
        ChatItem::Bubble(b) if !b.mine && !b.streaming && !b.blocks.is_empty() => {
            Some(format!("{:?}", b.blocks))
        }
        _ => None,
    })
}

#[tokio::test]
async fn controller_send_and_stream_roundtrip() {
    let mut r = rig().await;
    r.say("hello").await;
    let v = r
        .pump("agent reply", |v| {
            agent_text(v).is_some() && v.typing.is_none()
        })
        .await;
    assert!(agent_text(&v).is_some_and(|t| t.contains("Accepted")));
    assert!(v.typing.is_none() && !v.can_stop);
}

#[tokio::test]
async fn controller_ask_allow_roundtrip_and_journal() {
    let mut r = rig().await;
    r.say("/ask send a@example.org").await;
    let v = r
        .pump("ask card", |v| {
            ask_card(v).is_some_and(|c| c.1 == CardState::Pending)
        })
        .await;
    let (id, _) = ask_card(&v).expect("card");
    r.click(id, ButtonKind::Allow).await;
    r.pump("card resolved", |v| {
        ask_card(v).is_some_and(|c| c.1 == CardState::Allowed)
    })
    .await;
    let log = r.http.list_actions(None).await.unwrap_or_default();
    assert!(log["items"].as_array().is_some_and(|a| {
        a.iter()
            .any(|x| x["id"] == id && x["decision"] == "ask-approved")
    }));
    let again = r.chat.update(
        ChatMsg::CardButton {
            request_id: id,
            button: ButtonKind::Deny,
        },
        Instant::now(),
    );
    assert!(again.is_empty(), "a repeated answer does not go out");
}

#[tokio::test]
async fn controller_second_decision_409_shows_winner() {
    let mut r = rig().await;
    r.say("/ask delete x").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    assert!(decide_outcome(&r.http, id, PermDecision::Deny, None).await == DecisionOutcome::Done);
    r.click(id, ButtonKind::Allow).await;
    r.pump("the card takes the state of whoever decided", |v| {
        ask_card(v).is_some_and(|c| c.1 == CardState::Denied)
    })
    .await;
}

#[tokio::test]
async fn controller_observer_dangerous_cmd_flow() {
    let mut r = rig().await;
    r.say("/danger rm -rf /").await;
    let v = r
        .pump("observer card", |v| observer(v).is_some_and(|c| c.awaiting))
        .await;
    let card = observer(&v).expect("card");
    assert!(card.red_border);
    r.click(card.request_id, ButtonKind::Reject).await;
    r.pump("decision accepted", |v| {
        observer(v).is_some_and(|c| !c.awaiting)
    })
    .await;
    let f = r
        .http
        .list_findings(Some("dangerous_cmd"))
        .await
        .unwrap_or_default();
    assert!(f["items"].as_array().is_some_and(|a| !a.is_empty()), "{f}");
}

#[tokio::test]
async fn controller_observer_unavailable_fail_closed() {
    let mut r = rig().await;
    r.d.set_observer_available(false);
    r.say("/danger rm -rf /").await;
    let v = r.pump("observer card", |v| observer(v).is_some()).await;
    let card = observer(&v).expect("card");
    assert!(
        card.detail
            .as_deref()
            .is_some_and(|d| d.contains("unavailable")),
        "{card:?}"
    );
    assert_eq!(card.verdict, controllers::chat_view::Verdict::Blocked);
    assert!(
        card.buttons.is_empty(),
        "a blocked action cannot be confirmed"
    );
}

#[tokio::test]
async fn controller_html_reply_shown_literally() {
    let mut r = rig().await;
    r.say("/html").await;
    let v = r.pump("reply with HTML", |v| agent_text(v).is_some()).await;
    let shown = agent_text(&v).unwrap_or_default();
    assert!(shown.contains("<"), "markup stays literal text: {shown}");
}

#[tokio::test]
async fn persona_chip_roundtrip() {
    let mut r = rig().await;
    let fx = r.chat.update(ChatMsg::PickPersona(Some(2)), Instant::now());
    r.effects(fx).await;
    let s = r.http.list_sessions().await.unwrap_or_default();
    let row = s.iter().find(|x| x["id"] == SESSION);
    assert!(
        row.is_some_and(|x| x["persona_id"] == 2 && x["effective_persona_id"] == 2),
        "{row:?}"
    );
    r.pump("chip shows the persona", |v| {
        v.chips
            .as_ref()
            .is_some_and(|c| c.persona_label.contains("Work"))
    })
    .await;
}

#[tokio::test]
async fn secrets_not_in_client_logs() {
    let r = rig().await;
    let token = r.d.admin_token().to_string();
    let shown = format!("{:?} {:?} {:?}", r.chat.view(), r.http, r.c.events());
    assert!(
        !shown.contains(&token),
        "the token must not appear in the Debug of the client and the view"
    );
}
