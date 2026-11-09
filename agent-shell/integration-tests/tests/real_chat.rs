//! Chat scenarios of the GUI (ChatController + core-client) against a real `agentd` with the `scripted` provider.
//! The same scenarios as in `chat_controller.rs` against `fake-agentd`; contract differences are recorded in
//! `reports/S4-real-agentd-report.md`.
mod real_rig;

use std::time::Instant;

use controllers::chat_view::{ButtonKind, CardState, ChatItem, ChatView, ObserverCard};
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::permissions::{Mode, PermDecision, RuleScope};
use controllers::{Controller, CoreCommand, Effect};
use core_client::http::Request;
use core_client::{CoreClient, CoreEvent, HttpClient};
use real_rig::{Real, WAIT};
use serde_json::{Value, json};
use ui::chat_exec::{DecisionOutcome, decide_outcome, upload_all};
use ui::chat_feed::{apply_personas, route, topic};

struct Rig {
    r: Real,
    session: i64,
    c: CoreClient,
    http: HttpClient,
    chat: ChatController,
    rx: async_channel::Receiver<CoreEvent>,
}

async fn rig() -> Rig {
    let r = Real::start().await;
    let session = r.model_session().await;
    let c = r.client();
    let rx = c.events();
    let http = r.http.clone();
    let mut chat = ChatController::new();
    assert!(c.subscribe(&topic(session)).is_ok());
    let _ = chat.update(ChatMsg::Select(Some(session)), Instant::now());
    let list = http.list_personas().await.unwrap_or_default();
    let _ = apply_personas(&mut chat, &list, Instant::now());
    Rig {
        r,
        session,
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
                        .expect("attachments uploaded");
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

    async fn msg(&mut self, m: ChatMsg) {
        let fx = self.chat.update(m, Instant::now());
        self.effects(fx).await;
    }

    async fn pump(&mut self, what: &str, ok: impl Fn(&ChatView) -> bool) -> ChatView {
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            if ok(&self.chat.view()) {
                return self.chat.view();
            }
            if let Ok(Ok(ev)) =
                tokio::time::timeout(std::time::Duration::from_millis(100), self.rx.recv()).await
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

    fn actions(&self) -> Vec<Value> {
        self.r.actions()
    }

    async fn messages(&self) -> Vec<Value> {
        self.r
            .get(&format!("/sessions/{}/messages?limit=200", self.session))
            .await
            .as_array()
            .cloned()
            .unwrap_or_default()
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

fn bubbles(v: &ChatView, mine: bool) -> usize {
    v.items
        .iter()
        .filter(|i| matches!(i, ChatItem::Bubble(b) if b.mine == mine))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn real_send_and_stream_roundtrip() {
    let mut r = rig().await;
    r.say("hello").await;
    let v = r
        .pump("agent reply", |v| {
            agent_text(v).is_some() && v.typing.is_none()
        })
        .await;
    assert!(agent_text(&v).is_some_and(|t| t.contains("Reply: hello")));
    assert!(v.typing.is_none() && !v.can_stop);
    let rows = r.messages().await;
    assert!(
        rows.iter()
            .any(|m| m["role"] == "agent" && m["status"] == "done")
    );
    assert!(rows.iter().any(|m| m["role"] == "user"));
}

#[tokio::test(flavor = "multi_thread")]
async fn real_stop_midstream() {
    let mut r = rig().await;
    r.say("#scn:hang waiting").await;
    r.pump("stream running", |v| v.can_stop && v.typing.is_some())
        .await;
    r.msg(ChatMsg::Stop).await;
    let v = r
        .pump("stream stopped", |v| !v.can_stop && v.typing.is_none())
        .await;
    assert!(
        v.items
            .iter()
            .any(|i| matches!(i, ChatItem::Bubble(b) if !b.mine))
    );
    let rows = r.messages().await;
    assert!(
        rows.iter()
            .any(|m| m["role"] == "agent" && m["status"] == "stopped"),
        "{rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_stream_error_card() {
    let mut r = rig().await;
    r.say("#scn:drop x").await;
    r.pump("error card", |v| {
        v.items.iter().any(|i| matches!(i, ChatItem::Error(_)))
    })
    .await;
    // `typing:false` arrives as a separate frame after `message.done`.
    r.pump("indicator went out after the error", |v| v.typing.is_none())
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn real_ask_allow_roundtrip_and_journal() {
    let mut r = rig().await;
    r.say("#scn:action_send hello").await;
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
    let log = r.actions();
    assert!(
        log.iter()
            .any(|x| x["action"] == "send" && x["decision"] == "ask-approved"),
        "{log:?}"
    );
    assert!(
        log.iter()
            .all(|x| x["action"] != "send" || x["reversible"] == 0 || x["reversible"] == false),
        "send is irreversible: {log:?}"
    );
    assert!(
        r.chat
            .update(
                ChatMsg::CardButton {
                    request_id: id,
                    button: ButtonKind::Deny
                },
                Instant::now()
            )
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_ask_deny_roundtrip() {
    let mut r = rig().await;
    r.say("#scn:action_delete x").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    r.click(id, ButtonKind::Deny).await;
    r.pump("card rejected", |v| {
        ask_card(v).is_some_and(|c| c.1 == CardState::Denied)
    })
    .await;
    let log = r.actions();
    assert!(
        log.iter()
            .any(|x| x["action"] == "delete" && x["decision"] == "ask-denied"),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_second_decision_409_shows_winner() {
    let mut r = rig().await;
    r.say("#scn:action_delete x").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    assert!(decide_outcome(&r.http, id, PermDecision::Deny, None).await == DecisionOutcome::Done);
    r.click(id, ButtonKind::Allow).await;
    r.pump("the card takes the state of whoever decided", |v| {
        ask_card(v).is_some_and(|c| c.1 == CardState::Denied)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn real_chat_token_cannot_decide() {
    let mut r = rig().await;
    r.say("#scn:action_send x").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    let chat = HttpClient::new(
        r.r.endpoint(),
        Some(core_client::Token::new(r.r.chat_token.clone())),
    )
    .with_timeout(WAIT);
    let res = ui::chat_exec::decide(&chat, id, PermDecision::Allow, None).await;
    assert_eq!(res.err().and_then(|e| e.api().map(|a| a.status)), Some(403));
    let req = r.http.permission_request(id).await.unwrap_or_default();
    assert_eq!(req["status"], "pending");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_ask_always_persists_permission() {
    let mut r = rig().await;
    let persona =
        r.r.post("/personas", json!({"name": "P", "color": "#336699"}))
            .await["id"]
            .as_i64()
            .expect("persona");
    let list = r.http.list_personas().await.unwrap_or_default();
    let _ = apply_personas(&mut r.chat, &list, Instant::now());
    r.msg(ChatMsg::PickPersona(Some(persona))).await;
    // Without a `shell` rule on the persona the core runs it without asking: ask for confirmation.
    let (st, b) =
        r.r.raw(Request::put(
            &format!("/v1/permissions/persona/{persona}/shell"),
            json!({"mode": "ask"}),
        ))
        .await;
    assert_eq!(st, 200, "{b}");
    r.say("#scn:action_shell ls").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    r.click(id, ButtonKind::Always).await;
    r.msg(ChatMsg::AlwaysPick {
        request_id: id,
        scope: RuleScope::Persona,
        mode: Mode::Allow,
    })
    .await;
    r.pump("card resolved", |v| {
        ask_card(v).is_some_and(|c| c.1 != CardState::Pending)
    })
    .await;
    let rows =
        r.r.get(&format!("/permissions?scope=persona&scope_id={persona}"))
            .await;
    assert_eq!(
        rows,
        json!([{"scope": "persona", "scope_id": persona, "action": "shell", "mode": "allow"}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_observer_dangerous_cmd_blocked_without_card() {
    // The core blocks a dangerous command at once (`blocked`), there is no confirmation card: the tool message and the finding show it.
    let mut r = rig().await;
    r.say("#scn:action_danger rm").await;
    r.pump("reply after the block", |v| {
        v.typing.is_none()
            && v.items.iter().any(|i| {
                matches!(i, ChatItem::Bubble(b) if !b.mine && format!("{:?}", b.blocks).contains("blocked_by_observer"))
            })
    })
    .await;
    assert!(observer(&r.chat.view()).is_none());
    let f = r.r.findings("dangerous_cmd");
    assert!(f.iter().any(|x| x["verdict"] == "blocked"), "{f:?}");
    let log = r.actions();
    assert!(
        log.iter()
            .any(|x| x["action"] == "shell" && x["decision"] == "deny"),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_observer_unavailable_card_fail_closed() {
    let mut r = rig().await;
    for a in ["delete", "shell"] {
        let (st, b) =
            r.r.raw(Request::put(
                &format!("/v1/permissions/global/0/{a}"),
                json!({"mode": "allow"}),
            ))
            .await;
        assert_eq!(st, 200, "{b}");
    }
    let (st, b) =
        r.r.raw(Request::post(
            "/v1/_test/observer",
            json!({"available": false}),
        ))
        .await;
    assert_eq!(st, 200, "{b}");
    r.say("#scn:action_delete notes").await;
    let v = r
        .pump("observer card", |v| observer(v).is_some_and(|c| c.awaiting))
        .await;
    let card = observer(&v).expect("card");
    assert!(card.red_border);
    assert!(
        card.detail
            .as_deref()
            .is_some_and(|d| d.contains("unavailable")),
        "{card:?}"
    );
    r.click(card.request_id, ButtonKind::Reject).await;
    r.pump("decision accepted", |v| {
        observer(v).is_some_and(|c| !c.awaiting)
    })
    .await;
    let log = r.actions();
    assert!(
        log.iter()
            .any(|x| x["action"] == "delete" && x["decision"] == "ask-denied"),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_html_reply_shown_literally() {
    let mut r = rig().await;
    r.say("#scn:html").await;
    let v = r.pump("reply with HTML", |v| agent_text(v).is_some()).await;
    let shown = agent_text(&v).unwrap_or_default();
    assert!(shown.contains("<"), "markup stays literal text: {shown}");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_persona_chip_roundtrip() {
    let mut r = rig().await;
    let p =
        r.r.post("/personas", json!({"name": "Work", "color": "#336699"}))
            .await["id"]
            .as_i64()
            .expect("persona");
    let list = r.http.list_personas().await.unwrap_or_default();
    let _ = apply_personas(&mut r.chat, &list, Instant::now());
    r.msg(ChatMsg::PickPersona(Some(p))).await;
    let s = r.http.list_sessions().await.unwrap_or_default();
    let row = s.iter().find(|x| x["id"] == r.session);
    assert!(
        row.is_some_and(|x| x["persona_id"] == p && x["effective_persona_id"] == p),
        "{row:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_path_chip_roundtrip() {
    let mut r = rig().await;
    // The core accepts only a path inside the home directory (the core's HOME = `<tmp>/home`), without symlinks pointing outside.
    let dir = std::fs::canonicalize(r.r.dir.path().join("home"))
        .expect("home")
        .join("work");
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.to_string_lossy().to_string();
    r.msg(ChatMsg::PickPath(Some(path.clone()))).await;
    let s = r.http.list_sessions().await.unwrap_or_default();
    let row = s.iter().find(|x| x["id"] == r.session);
    assert!(row.is_some_and(|x| x["path"] == path), "{row:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_attachment_roundtrip() {
    let mut r = rig().await;
    let file = r.r.dir.path().join("<i>note<i>.txt");
    std::fs::write(&file, "content").expect("file");
    r.msg(ChatMsg::AttachPicked {
        path: file.to_string_lossy().to_string(),
        name: "<i>note<i>.txt".into(),
        size: 20,
    })
    .await;
    r.say("with a file").await;
    let v = r
        .pump("bubble with an attachment", |v| {
            v.items
                .iter()
                .any(|i| matches!(i, ChatItem::Bubble(b) if b.mine && !b.attachments.is_empty()))
        })
        .await;
    let name = v.items.iter().find_map(|i| match i {
        ChatItem::Bubble(b) if b.mine => b.attachments.first().map(|a| a.name.clone()),
        _ => None,
    });
    assert_eq!(name.as_deref(), Some("<i>note<i>.txt"));
    let mut rows = r.messages().await;
    for _ in 0..50 {
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        rows = r.messages().await;
    }
    assert!(
        rows.iter()
            .any(|m| m["role"] == "user" && !m["meta"]["attachments"].is_null()),
        "{rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_duplicate_client_id_idempotent() {
    let r = rig().await;
    let cid = uuid::Uuid::now_v7().to_string();
    let a = r.http.send_message(r.session, &cid, "one", &[]).await;
    let b = r.http.send_message(r.session, &cid, "one", &[]).await;
    assert!(a.is_ok() && b.is_ok(), "{a:?} {b:?}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let users = r
        .messages()
        .await
        .into_iter()
        .filter(|m| m["role"] == "user")
        .count();
    assert_eq!(users, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn real_session_delete_keeps_journal() {
    let mut r = rig().await;
    r.say("#scn:action_send x").await;
    let v = r.pump("ask card", |v| ask_card(v).is_some()).await;
    let (id, _) = ask_card(&v).expect("card");
    r.click(id, ButtonKind::Allow).await;
    r.pump("decided", |v| {
        ask_card(v).is_some_and(|c| c.1 == CardState::Allowed)
    })
    .await;
    assert!(r.http.delete_session(r.session).await.is_ok());
    let log = r.actions();
    assert!(!log.is_empty(), "the journal survived session deletion");
    assert!(log.iter().all(|x| x["session_id"].is_null()), "{log:?}");
    let (st, _) =
        r.r.raw(Request::get(&format!(
            "/v1/sessions/{}/messages",
            r.session
        )))
        .await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn real_three_messages_all_delivered_in_order() {
    let mut r = rig().await;
    for t in ["one", "two", "three"] {
        r.say(t).await;
    }
    r.pump("three replies", |v| {
        bubbles(v, true) == 3 && bubbles(v, false) == 3 && v.typing.is_none()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn real_secrets_not_in_logs() {
    let mut r = rig().await;
    r.say("hello").await;
    r.pump("reply", |v| agent_text(v).is_some()).await;
    let token = r.r.admin_token.clone();
    let shown = format!("{:?} {:?} {:?}", r.chat.view(), r.http, r.c.events());
    assert!(!shown.contains(&token));
    assert!(!r.r.log_text().contains(&token), "token in the core log");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_core_crash_mid_stream_interrupted_and_reconnect() {
    let mut r = rig().await;
    r.say("#scn:trickle x").await;
    r.pump("stream running", |v| v.can_stop).await;
    r.r.crash_and_restart().await;
    // The client reconnects by itself; the snapshot carries a message with status `interrupted`.
    r.pump("reply interrupted", |v| {
        v.typing.is_none()
            && v.items.iter().any(|i| {
                matches!(i, ChatItem::Bubble(b) if !b.mine && b.note.is_some())
                    || matches!(i, ChatItem::Error(_))
            })
    })
    .await;
    let rows = r.messages().await;
    assert!(
        rows.iter()
            .any(|m| m["role"] == "agent" && m["status"] == "interrupted"),
        "{rows:?}"
    );
    // After recovery the chat works.
    r.say("again").await;
    r.pump("reply after the restart", |v| {
        v.items.iter().any(|i| {
            matches!(i, ChatItem::Bubble(b) if !b.mine && format!("{:?}", b.blocks).contains("Reply: again"))
        })
    })
    .await;
}
