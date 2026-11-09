//! Client core: WebSocket with reconnects, subscriptions, `controllers` and execution of their effects over REST.
//! Everything is single-threaded (`Rc`, `RefCell`): the page lives in one browser thread.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use controllers::bar_cluster::{BarCluster, BarClusterView, FrameKind};
use controllers::chat_view::ChatView;
use controllers::controller::CoreCommand;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::time::Instant;
use controllers::timezone::Zone;
use controllers::tree_types::{parse_delta, parse_personas, parse_snapshot};
use controllers::tree_view::TreeView;
use controllers::{BusEvent, ConnState, Controller, Effect};
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::api;
use crate::attach;
use crate::indicators::{self, Ind};
use crate::notes::{self, Note};
use crate::outbox::{self, Outgoing};
use crate::term::Term;
use crate::util::{unix_now, utc_offset_secs};
use controllers::chat_types::ReqStatus;
use controllers::permissions::PermDecision;

type Local<T> = RwSignal<T, LocalStorage>;

/// What the UI sees; updated after every controller step.
#[derive(Clone, Copy)]
pub struct Sig {
    pub conn: Local<ConnState>,
    pub bar: Local<BarClusterView>,
    pub tree: Local<TreeView>,
    pub chat: Local<ChatView>,
    /// Token rejected or access denied: show the sign-in form with the reason.
    pub denied: Local<Option<String>>,
    /// Notification feed, newest first.
    pub notes: Local<Vec<Note>>,
    /// Short message to the user (decision not delivered, missing permission, etc.).
    pub notice: Local<Option<String>>,
    /// How many messages are waiting for the connection to return.
    pub queued: Local<usize>,
    /// Terminal output of the agent of the selected session (`terminal.*` frames).
    pub term: Local<Term>,
    /// Bar indicators (`bar.indicators`).
    pub inds: Local<Vec<Ind>>,
    /// Consent to system browser notifications (only on explicit user action).
    pub popups: Local<bool>,
    /// Popup about a new event (disappears by itself).
    pub toast: Local<Option<String>>,
}

impl Sig {
    pub fn new() -> Sig {
        let bar = BarCluster::new(Zone::fixed(utc_offset_secs())).view();
        Sig {
            conn: RwSignal::new_local(ConnState::Connecting),
            bar: RwSignal::new_local(bar),
            tree: RwSignal::new_local(SessionTreeController::new().view()),
            chat: RwSignal::new_local(ChatController::new().view()),
            denied: RwSignal::new_local(None),
            notes: RwSignal::new_local(Vec::new()),
            notice: RwSignal::new_local(None),
            queued: RwSignal::new_local(0),
            term: RwSignal::new_local(Term::default()),
            inds: RwSignal::new_local(Vec::new()),
            popups: RwSignal::new_local(false),
            toast: RwSignal::new_local(None),
        }
    }
}

pub struct Core {
    token: String,
    sig: Sig,
    ws: RefCell<Option<WebSocket>>,
    subs: RefCell<BTreeSet<String>>,
    chat: RefCell<ChatController>,
    tree: RefCell<SessionTreeController>,
    bar: RefCell<BarCluster>,
    selected: Cell<Option<i64>>,
    pending_select: Cell<Option<i64>>,
    /// Persona of new sessions: the first from `GET /personas` (it has a model set).
    default_persona: Cell<Option<i64>>,
    /// A send that did not reach the core because of the network: retried after the connection returns (`client_id` makes it idempotent).
    outbox: RefCell<Vec<Outgoing>>,
    attempts: Cell<u32>,
    stopped: Cell<bool>,
}

fn now() -> Instant {
    Instant::now()
}

impl Core {
    pub fn new(token: String, sig: Sig) -> Rc<Core> {
        Rc::new(Core {
            token,
            sig,
            ws: RefCell::new(None),
            subs: RefCell::new(BTreeSet::new()),
            chat: RefCell::new(ChatController::new().with_utc_offset(utc_offset_secs())),
            tree: RefCell::new(SessionTreeController::new()),
            bar: RefCell::new(BarCluster::new(Zone::fixed(utc_offset_secs()))),
            selected: Cell::new(None),
            pending_select: Cell::new(None),
            default_persona: Cell::new(None),
            outbox: RefCell::new(crate::util::load_outbox()),
            attempts: Cell::new(0),
            stopped: Cell::new(false),
        })
    }

    /// Token check over REST, then WebSocket and clock. A `401`/`403` rejection shows the sign-in form.
    pub fn start(self: &Rc<Self>) {
        self.sig.queued.set(self.outbox.borrow().len());
        // The `Authorization` header does not accept non-ASCII: the core never issued such a token.
        if !self.token.is_ascii() {
            self.deny("Token not accepted");
            return;
        }
        let me = self.clone();
        spawn_local(async move {
            match api::request("GET", "/v1/folders", &me.token, None).await {
                Ok(r) if r.status == 401 => me.deny("Token not accepted"),
                Ok(r) if r.status == 429 => me.deny(&api::status_text(429, r.retry_after)),
                Ok(r) if r.status == 403 => {
                    me.deny("Access denied: page origin not allowed by the core")
                }
                _ => {
                    me.open_ws();
                    me.clone().start_clock();
                }
            }
        });
    }

    pub fn stop(&self) {
        self.stopped.set(true);
        if let Some(ws) = self.ws.borrow_mut().take() {
            drop(ws.close());
        }
    }

    fn deny(&self, why: &str) {
        self.stopped.set(true);
        self.sig.denied.set(Some(why.to_string()));
    }

    fn start_clock(self: Rc<Self>) {
        let me = self.clone();
        let ticks = Cell::new(0u32);
        let tick = Closure::<dyn FnMut()>::new(move || {
            if me.stopped.get() {
                return;
            }
            // A connection may hang without a close event (network drop): the queue tries to flush every 5 seconds.
            ticks.set(ticks.get().wrapping_add(1));
            if ticks.get().is_multiple_of(5) && !me.outbox.borrow().is_empty() {
                me.flush_outbox();
            }
            let fx = me.bar.borrow_mut().tick(unix(), now());
            drop(fx);
            me.refresh();
        });
        if let Some(w) = web_sys::window() {
            drop(w.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                1000,
            ));
        }
        tick.forget();
    }

    fn set_conn(&self, state: ConnState) {
        self.sig.conn.set(state);
        let fx = self.bar.borrow_mut().on_connection(state, now());
        drop(fx);
        self.refresh();
    }

    fn open_ws(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let Some(loc) = web_sys::window().map(|w| w.location()) else {
            return;
        };
        let scheme = if loc.protocol().is_ok_and(|p| p == "https:") {
            "wss"
        } else {
            "ws"
        };
        let host = loc.host().unwrap_or_default();
        let protocols = js_sys::Array::new();
        protocols.push(&JsValue::from_str("agentd.v1"));
        protocols.push(&JsValue::from_str(&format!("bearer.{}", self.token)));
        let Ok(ws) =
            WebSocket::new_with_str_sequence(&format!("{scheme}://{host}/v1/ws"), &protocols)
        else {
            self.set_conn(ConnState::Offline);
            return;
        };
        self.set_conn(if self.attempts.get() == 0 {
            ConnState::Connecting
        } else {
            ConnState::Reconnecting
        });
        let me = self.clone();
        let on_open = Closure::<dyn FnMut()>::new(move || {
            me.attempts.set(0);
            me.set_conn(ConnState::Connected);
            let topics: Vec<String> = me.subs.borrow().iter().cloned().collect();
            for t in topics {
                me.send(&json!({"op": "subscribe", "topic": t}));
            }
            me.subscribe("sessions");
            me.subscribe("bar.quota");
            me.subscribe("bar.weather");
            me.subscribe("bar.sys");
            me.subscribe("bar.indicators");
            me.subscribe("events");
            me.flush_outbox();
            me.list_personas();
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();
        let me = self.clone();
        let on_msg = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            if let Some(t) = e.data().as_string() {
                me.on_wire(&t);
            }
        });
        ws.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();
        let me = self.clone();
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            me.ws.borrow_mut().take();
            match e.code() {
                4401 => me.deny("Token revoked or not accepted"),
                4403 => me.deny("Access denied: page origin not allowed by the core"),
                4426 => me.deny("Client version not supported by the core"),
                _ => me.reconnect(),
            }
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();
        *self.ws.borrow_mut() = Some(ws);
    }

    fn reconnect(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let n = self.attempts.get();
        self.attempts.set(n + 1);
        self.set_conn(ConnState::Reconnecting);
        let delay = 1000 * (1u32 << n.min(4)).min(15);
        let me = self.clone();
        let cb = Closure::once_into_js(move || me.open_ws());
        if let Some(w) = web_sys::window() {
            drop(w.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.unchecked_ref(),
                i32::try_from(delay).unwrap_or(15_000),
            ));
        }
    }

    fn send(&self, v: &Value) {
        if let Some(ws) = self.ws.borrow().as_ref()
            && ws.ready_state() == WebSocket::OPEN
        {
            drop(ws.send_with_str(&v.to_string()));
        }
    }

    fn subscribe(&self, topic: &str) {
        self.subs.borrow_mut().insert(topic.to_string());
        self.send(&json!({"op": "subscribe", "topic": topic}));
    }

    fn unsubscribe(&self, topic: &str) {
        self.subs.borrow_mut().remove(topic);
        self.send(&json!({"op": "unsubscribe", "topic": topic}));
    }

    fn refresh(&self) {
        self.sig.bar.set(self.bar.borrow().view());
        self.sig.tree.set(self.tree.borrow().view());
        self.sig.chat.set(self.chat.borrow().view());
    }

    fn on_wire(self: &Rc<Self>, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let (Some(topic), Some(kind)) = (v["topic"].as_str(), v["type"].as_str()) else {
            return;
        };
        let data = &v["data"];
        let frame = match kind {
            "snapshot" => FrameKind::Snapshot,
            "delta" => FrameKind::Delta,
            _ => FrameKind::Event,
        };
        let fx = match topic {
            "sessions" => {
                let msg = match frame {
                    FrameKind::Snapshot => parse_snapshot(data)
                        .map(|(folders, sessions)| TreeMsg::Snapshot { folders, sessions }),
                    FrameKind::Delta => parse_delta(data).map(TreeMsg::Frame),
                    FrameKind::Event => None,
                };
                let fx = msg.map_or_else(Vec::new, |m| self.tree.borrow_mut().update(m, now()));
                self.try_pending_select();
                fx
            }
            "events" => {
                self.on_events(frame, kind, data);
                Vec::new()
            }
            "bar.indicators" => {
                let mut list = self.sig.inds.get_untracked();
                match frame {
                    FrameKind::Snapshot => list = indicators::parse_snapshot(data),
                    _ => indicators::apply_delta(&mut list, data),
                }
                self.sig.inds.set(list);
                Vec::new()
            }
            t if t.starts_with("bar.") => {
                self.bar
                    .borrow_mut()
                    .on_frame(t, frame, data, unix(), now())
            }
            t => match t
                .strip_prefix("session.")
                .and_then(|s| s.parse::<i64>().ok())
            {
                Some(_)
                    if kind == "event"
                        && data["kind"]
                            .as_str()
                            .is_some_and(|k| k.starts_with("terminal.")) =>
                {
                    let mut term = self.sig.term.get_untracked();
                    term.on_frame(data);
                    self.sig.term.set(term);
                    Vec::new()
                }
                Some(session_id) => self.chat.borrow_mut().update(
                    ChatMsg::Frame {
                        session_id,
                        snapshot: frame == FrameKind::Snapshot,
                        data: data.clone(),
                    },
                    now(),
                ),
                None => Vec::new(),
            },
        };
        self.run(fx);
        self.refresh();
    }

    fn try_pending_select(self: &Rc<Self>) {
        if let Some(id) = self.pending_select.get()
            && self.tree.borrow().session_info(id).is_some()
        {
            self.pending_select.set(None);
            let fx = self.tree.borrow_mut().update(TreeMsg::Select(id), now());
            self.run(fx);
        }
    }

    pub fn tree_msg(self: &Rc<Self>, msg: TreeMsg) {
        let fx = self.tree.borrow_mut().update(msg, now());
        self.run(fx);
        self.refresh();
    }

    pub fn chat_msg(self: &Rc<Self>, msg: ChatMsg) {
        let fx = self.chat.borrow_mut().update(msg, now());
        self.run(fx);
        self.refresh();
    }

    fn open_session(self: &Rc<Self>, id: i64) {
        if let Some(prev) = self.selected.replace(Some(id)) {
            if prev == id {
                return;
            }
            self.unsubscribe(&format!("session.{prev}"));
        }
        self.sig.term.set(Term::default());
        self.chat_msg(ChatMsg::Select(Some(id)));
        self.subscribe(&format!("session.{id}"));
        self.post(&format!("/v1/sessions/{id}/read"), json!({}));
    }

    pub fn create_session(self: &Rc<Self>) {
        let me = self.clone();
        spawn_local(async move {
            let body = json!({"name": "New session", "folder_id": null, "persona_id": me.default_persona.get()});
            if let Ok(r) = api::request("POST", "/v1/sessions", &me.token, Some(&body)).await
                && r.ok()
                && let Some(id) = r.body["id"].as_i64()
            {
                me.pending_select.set(Some(id));
                me.try_pending_select();
            }
        });
    }

    fn post(self: &Rc<Self>, path: &str, body: Value) {
        let (me, path) = (self.clone(), path.to_string());
        spawn_local(async move {
            drop(api::request("POST", &path, &me.token, Some(&body)).await);
        });
    }

    fn list_personas(self: &Rc<Self>) {
        let me = self.clone();
        spawn_local(async move {
            if let Ok(r) = api::request("GET", "/v1/personas", &me.token, None).await
                && r.ok()
            {
                let list = parse_personas(&r.body);
                me.default_persona.set(list.first().map(|p| p.id));
                let fx = me
                    .tree
                    .borrow_mut()
                    .update(TreeMsg::Personas(list.clone()), now());
                let fx2 = me.chat.borrow_mut().update(ChatMsg::Personas(list), now());
                me.run(fx);
                me.run(fx2);
                me.refresh();
            }
        });
    }

    /// Send a message: network down means queue (retry after reconnect), a core `4xx` means "not sent".
    fn send_out(self: &Rc<Self>, out: Outgoing) {
        let me = self.clone();
        spawn_local(async move {
            let mut body = json!({"body_md": out.body_md, "client_id": out.client_id});
            if !out.attachment_ids.is_empty() {
                body["attachment_ids"] = json!(out.attachment_ids);
            }
            let path = format!("/v1/sessions/{}/messages", out.session_id);
            match api::request("POST", &path, &me.token, Some(&body)).await {
                Ok(r) if r.ok() => {
                    me.outbox
                        .borrow_mut()
                        .retain(|o| o.client_id != out.client_id);
                    me.sync_queued();
                }
                Ok(r) if r.status >= 500 || r.status == 429 => {
                    if r.status == 429 {
                        me.notice(&api::status_text(429, r.retry_after));
                    }
                    me.queue(out)
                }
                Ok(_) => {
                    me.outbox
                        .borrow_mut()
                        .retain(|o| o.client_id != out.client_id);
                    me.sync_queued();
                    me.chat_msg(ChatMsg::SendFailed {
                        client_id: out.client_id,
                    });
                }
                Err(_) => me.queue(out),
            }
        });
    }

    fn queue(&self, out: Outgoing) {
        let mut q = self.outbox.borrow_mut();
        if !q.iter().any(|o| o.client_id == out.client_id) {
            q.push(out);
        }
        drop(q);
        self.sync_queued();
    }

    /// After the connection returns the queue flushes in order; the core accepts a repeated `client_id` as the same request.
    fn flush_outbox(self: &Rc<Self>) {
        let queued: Vec<Outgoing> = self.outbox.borrow().clone();
        for o in queued {
            self.send_out(o);
        }
    }

    fn sync_queued(&self) {
        self.sig.queued.set(self.outbox.borrow().len());
        crate::util::store_outbox(&outbox::encode(&self.outbox.borrow()));
    }

    fn decide(self: &Rc<Self>, request_id: i64, decision: PermDecision) {
        let name = match decision {
            PermDecision::Allow => "allow",
            PermDecision::Deny => "deny",
            // "Always" changes permission rules: not decided from the browser, the card returns to the question.
            PermDecision::Always => {
                self.notice("“Always” is decided in the native GUI");
                self.chat_msg(ChatMsg::DecisionFailed { request_id });
                return;
            }
        };
        let me = self.clone();
        spawn_local(async move {
            let path = format!("/v1/permission-requests/{request_id}/decision");
            match api::request("POST", &path, &me.token, Some(&json!({"decision": name}))).await {
                Ok(r) if r.ok() => {}
                Ok(r) if r.status == 409 => {
                    let st = api::request(
                        "GET",
                        &format!("/v1/permission-requests/{request_id}"),
                        &me.token,
                        None,
                    )
                    .await
                    .ok()
                    .and_then(|r| r.body["status"].as_str().and_then(ReqStatus::parse));
                    match st {
                        Some(status) => {
                            me.chat_msg(ChatMsg::DecisionRejected { request_id, status })
                        }
                        None => me.chat_msg(ChatMsg::DecisionFailed { request_id }),
                    }
                }
                Ok(r) => {
                    me.notice(if r.status == 403 {
                        "No permission to decide requests: a web token with the decide scope is needed"
                    } else {
                        "Decision not accepted by the core"
                    });
                    me.chat_msg(ChatMsg::DecisionFailed { request_id });
                }
                Err(_) => {
                    me.notice("Decision not delivered: no connection");
                    me.chat_msg(ChatMsg::DecisionFailed { request_id });
                }
            }
        });
    }

    /// Short hint about an event; removed after 6 seconds.
    fn show_toast(&self, text: &str) {
        self.sig.toast.set(Some(text.to_string()));
        let toast = self.sig.toast;
        let text = text.to_string();
        let cb = Closure::once_into_js(move || {
            if toast.get_untracked().as_deref() == Some(text.as_str()) {
                toast.set(None);
            }
        });
        if let Some(w) = web_sys::window() {
            drop(w.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 6000));
        }
    }

    pub fn notice(&self, text: &str) {
        self.sig.notice.set(Some(text.to_string()));
    }

    fn on_events(&self, frame: FrameKind, kind: &str, data: &Value) {
        let mut list = self.sig.notes.get_untracked();
        match (frame, kind) {
            (FrameKind::Snapshot, _) => list = notes::parse_list(data),
            (_, "event") => {
                if let Some(n) = notes::parse_note(data) {
                    let hidden = web_sys::window()
                        .and_then(|w| w.document())
                        .is_some_and(|d| d.visibility_state() == web_sys::VisibilityState::Hidden);
                    if notes::should_popup(&n, self.sig.popups.get_untracked(), hidden) {
                        popup(&n.text);
                    }
                    if n.unread {
                        self.show_toast(&n.text);
                    }
                    notes::push(&mut list, n);
                }
            }
            _ => {
                if let Some(ids) = data.get("read").and_then(Value::as_array) {
                    let ids: Vec<i64> = ids.iter().filter_map(Value::as_i64).collect();
                    notes::mark_read(&mut list, &ids);
                }
            }
        }
        self.sig.notes.set(list);
    }

    /// Click on a notification: mark it read and open the session.
    pub fn read_note(self: &Rc<Self>, id: i64) {
        let mut list = self.sig.notes.get_untracked();
        notes::mark_read(&mut list, &[id]);
        self.sig.notes.set(list);
        self.post(&format!("/v1/events/{id}/read"), json!({}));
    }

    pub fn open_note(self: &Rc<Self>, id: i64, session_id: Option<i64>) {
        self.read_note(id);
        if let Some(s) = session_id {
            self.pending_select.set(Some(s));
            self.try_pending_select();
        }
    }

    /// Open a session from search or a notification (once it appears in the tree).
    pub fn goto_session(self: &Rc<Self>, id: i64) {
        self.pending_select.set(Some(id));
        self.try_pending_select();
    }

    /// Request with a body for windows without a controller (tasks): `Err` is a reason without token values.
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<(), String> {
        match api::request(method, path, &self.token, body.as_ref()).await {
            Ok(r) if r.ok() => Ok(()),
            Ok(r) => Err(api::status_text(r.status, r.retry_after)),
            Err(e) => Err(e),
        }
    }

    /// Re-read the session: the core sends no frame for a history clear, the snapshot comes from a new subscription.
    pub fn resync_session(&self, id: i64) {
        let t = format!("session.{id}");
        self.unsubscribe(&t);
        self.subscribe(&t);
    }

    /// Selected session (for tasks and attachments).
    pub fn selected(&self) -> Option<i64> {
        self.selected.get()
    }

    /// Settings page with `rev`: `(document, rev)`.
    pub async fn get_doc(&self, path: &str) -> Result<(Value, Option<i64>), String> {
        match api::request("GET", path, &self.token, None).await {
            Ok(r) if r.ok() => {
                let rev = crate::settings::doc_rev(&r.body)
                    .or_else(|| r.etag.as_deref().and_then(etag_rev));
                Ok((r.body, rev))
            }
            Ok(r) => Err(api::status_text(r.status, r.retry_after)),
            Err(e) => Err(e),
        }
    }

    /// `PATCH` with `If-Match`. Error: `(status, text, field errors)`; `412` means the page must be re-read.
    pub async fn patch_doc(
        &self,
        path: &str,
        rev: i64,
        patch: &Value,
    ) -> Result<(Value, Option<i64>), PatchFail> {
        let tag = format!("\"{rev}\"");
        match api::send(
            "PATCH",
            path,
            &self.token,
            api::Payload::Json(patch),
            &[("If-Match", &tag)],
        )
        .await
        {
            Ok(r) if r.ok() => {
                let new_rev = crate::settings::doc_rev(&r.body);
                Ok((r.body, new_rev))
            }
            Ok(r) => Err(PatchFail {
                status: r.status,
                text: api::status_text(r.status, r.retry_after),
                fields: crate::settings::api_field_errors(&r.body),
            }),
            Err(e) => Err(PatchFail {
                status: 0,
                text: e,
                fields: Vec::new(),
            }),
        }
    }

    /// Upload an attachment into the selected session and stage it in the draft (the controller stores the `id` as the path).
    pub fn upload_file(self: &Rc<Self>, file: web_sys::File) {
        let Some(sid) = self.selected.get() else {
            return;
        };
        let (name, size) = (file.name(), file.size() as u64);
        if let Err(why) = attach::check_file(&name, size) {
            self.notice(&format!("Attachment “{name}” not uploaded: {why}"));
            return;
        }
        let me = self.clone();
        spawn_local(async move {
            let Ok(form) = web_sys::FormData::new() else {
                return;
            };
            if form
                .append_with_blob_and_filename("file", &file, &name)
                .is_err()
            {
                return;
            }
            let path = format!("/v1/sessions/{sid}/attachments");
            match api::send("POST", &path, &me.token, api::Payload::Form(&form), &[]).await {
                Ok(r) if r.ok() => match attach::parse_uploaded(&r.body) {
                    Some(u) => me.chat_msg(ChatMsg::AttachPicked {
                        path: u.id.to_string(),
                        name: u.name,
                        size: u.size,
                    }),
                    None => me.notice("The core answered the upload without an id"),
                },
                Ok(r) => me.notice(&format!(
                    "Attachment “{name}” not uploaded: {}",
                    api::status_text(r.status, r.retry_after)
                )),
                Err(e) => me.notice(&format!("Attachment “{name}” not uploaded: {e}")),
            }
        });
    }

    pub fn read_all_notes(self: &Rc<Self>) {
        let mut list = self.sig.notes.get_untracked();
        let ids: Vec<i64> = list.iter().map(|n| n.id).collect();
        notes::mark_read(&mut list, &ids);
        self.sig.notes.set(list);
        self.post("/v1/events/read", json!({}));
    }

    /// GET for screens that need no controller (Settings): `Ok(body)` or a short reason.
    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        match api::request("GET", path, &self.token, None).await {
            Ok(r) if r.ok() => Ok(r.body),
            Ok(r) => Err(api::status_text(r.status, r.retry_after)),
            Err(e) => Err(e),
        }
    }

    fn run(self: &Rc<Self>, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::Publish(BusEvent::OpenSession { session_id, .. }) => {
                    self.open_session(session_id)
                }
                Effect::Command(c) => self.command(c),
                Effect::Publish(_) | Effect::Persist(_) => {}
            }
        }
    }

    fn command(self: &Rc<Self>, c: CoreCommand) {
        match c {
            CoreCommand::SendMessage {
                session_id,
                client_id,
                body_md,
                attachments,
            } => self.send_out(Outgoing {
                session_id,
                client_id,
                body_md,
                attachment_ids: attach::attachment_ids(
                    &attachments
                        .iter()
                        .map(|a| a.path.clone())
                        .collect::<Vec<_>>(),
                ),
            }),
            CoreCommand::DecidePermission {
                request_id,
                decision,
                ..
            } => {
                self.decide(request_id, decision);
            }
            CoreCommand::StopStream { session_id } => {
                self.post(&format!("/v1/sessions/{session_id}/stop"), json!({}));
            }
            CoreCommand::MarkRead { id } => {
                self.post(&format!("/v1/sessions/{id}/read"), json!({}))
            }
            CoreCommand::ListPersonas => self.list_personas(),
            CoreCommand::ResyncSession { session_id } => {
                let t = format!("session.{session_id}");
                self.unsubscribe(&t);
                self.subscribe(&t);
            }
            _ => {}
        }
    }
}

/// System browser notification; a failure (no permission) is silently skipped.
fn popup(text: &str) {
    let opts = web_sys::NotificationOptions::new();
    opts.set_body(text);
    drop(web_sys::Notification::new_with_options("Agent OS", &opts));
}

/// Request for system notification permission: called only from a user action.
pub fn request_popup_permission(sig: Sig) {
    let Ok(p) = web_sys::Notification::request_permission() else {
        return;
    };
    spawn_local(async move {
        let granted = wasm_bindgen_futures::JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .is_some_and(|v| v == "granted");
        sig.popups.set(granted);
    });
}

/// Failed settings edit.
#[derive(Debug, Clone)]
pub struct PatchFail {
    pub status: u16,
    pub text: String,
    pub fields: Vec<crate::settings::FieldErr>,
}

/// `"12"` → 12.
fn etag_rev(tag: &str) -> Option<i64> {
    tag.trim().trim_matches('"').parse().ok()
}

fn unix() -> i64 {
    unix_now()
}
