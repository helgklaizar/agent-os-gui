//! Shared stand of the notification feed tests against `fake-agentd`: core client, feed, tree, bus, effect execution.
#![allow(dead_code)]

use std::time::Instant;

use controllers::controllers::notifications::{NotifMsg, NotificationsController};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::{BusEvent, Controller, Effect};
use core_client::api::FrameKind;
use core_client::{CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use serde_json::Value;
use ui::notif_feed::{self, apply_subtypes};
use ui::{notif_exec, tree_exec, tree_feed};

use crate::common::{WAIT, client, http, start};

pub struct Rig {
    pub d: FakeAgentd,
    pub c: CoreClient,
    pub http: HttpClient,
    pub n: NotificationsController,
    pub tree: SessionTreeController,
    pub rx: async_channel::Receiver<CoreEvent>,
    /// All that the controllers published to the bus (in publication order).
    pub bus: Vec<BusEvent>,
    /// How many `events` snapshots arrived (grows after a reconnect).
    pub snapshots: usize,
}

/// A mock without event fixtures: the test fills the feed.
pub async fn rig() -> Rig {
    rig_with(Config::temp().no_ticker().no_event_fixtures()).await
}

pub async fn rig_with(config: Config) -> Rig {
    let d = start(config).await;
    let c = client(&d);
    let rx = c.events();
    for t in notif_feed::TOPICS.into_iter().chain(tree_feed::TOPICS) {
        assert!(c.subscribe(t).is_ok(), "subscription {t}");
    }
    let http = http(&d, Some(d.admin_token()));
    let mut r = Rig {
        d,
        c,
        http,
        n: NotificationsController::new(),
        tree: SessionTreeController::new(),
        rx,
        bus: Vec::new(),
        snapshots: 0,
    };
    r.load_subtypes().await;
    r.until("events and sessions snapshots", |r| {
        r.snapshots >= 1 && !r.tree.view().rows.is_empty()
    })
    .await;
    r
}

impl Rig {
    pub async fn load_subtypes(&mut self) {
        let list = self.http.event_subtypes().await.unwrap_or_default();
        let fx = apply_subtypes(&mut self.n, &list, Instant::now());
        self.keep(fx);
    }

    fn keep(&mut self, fx: Vec<Effect>) {
        for e in fx {
            if let Effect::Publish(b) = e {
                self.bus.push(b);
            }
        }
    }

    /// A core client event -> feed and tree; effects are returned, `run` executes them.
    pub fn feed(&mut self, ev: &CoreEvent) -> Vec<Effect> {
        let now = Instant::now();
        if let CoreEvent::Frame(f) = ev
            && f.topic == "events"
            && f.kind == FrameKind::Snapshot
        {
            self.snapshots += 1;
        }
        let routed = notif_feed::route(&mut self.n, ev, now);
        tree_feed::route(&mut self.tree, ev, now);
        routed.effects
    }

    /// Parses client events until a condition over the stand holds; feed commands are executed.
    pub async fn until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(self) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!(
                    "timed out waiting for: {what}; feed: {:?}",
                    self.n
                        .view()
                        .cards
                        .iter()
                        .map(|c| (c.id, c.unread))
                        .collect::<Vec<_>>()
                )
            };
            let fx = self.feed(&ev);
            self.run(fx).await;
        }
    }

    /// Executes effects: commands via core requests, `OpenSession` by selecting in the tree and in the feed (as the shell).
    pub async fn run(&mut self, effects: Vec<Effect>) -> Vec<Result<(), String>> {
        let mut pending: Vec<Effect> = effects.into_iter().rev().collect();
        let mut results = Vec::new();
        while let Some(e) = pending.pop() {
            match e {
                Effect::Command(cmd) => {
                    let done = match notif_exec::execute(&self.http, &cmd).await {
                        Some(r) => Some(r),
                        None => tree_exec::execute(&self.http, &cmd).await,
                    };
                    results.push(done.unwrap_or_else(|| Err(format!("not executed: {cmd:?}"))));
                }
                Effect::Publish(b) => {
                    self.bus.push(b.clone());
                    if let BusEvent::OpenSession { session_id, .. } = b {
                        let now = Instant::now();
                        let fx = self.tree.update(TreeMsg::Opened(session_id), now);
                        pending.extend(fx.into_iter().rev());
                        let fx = self.n.update(NotifMsg::Bus(b), now);
                        pending.extend(fx.into_iter().rev());
                    }
                }
                Effect::Persist(_) => {}
            }
        }
        results
    }

    pub async fn msg(&mut self, msg: NotifMsg) -> Vec<Result<(), String>> {
        let fx = self.n.update(msg, Instant::now());
        self.run(fx).await
    }

    /// Core events via SQL, independent of the mock and client code: `(id, kind, sub, session_id, unread)`.
    pub fn db_events(&self) -> Vec<(i64, String, String, Option<i64>, bool)> {
        let conn = self.d.db().reader().expect("reader");
        let mut st = conn
            .prepare("SELECT id, kind, sub, session_id, unread FROM events ORDER BY id")
            .expect("query");
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get::<_, i64>(4)? != 0,
            ))
        })
        .expect("rows")
        .flatten()
        .collect()
    }

    /// Unread events via SQL: by kind and (if given) subtype.
    pub fn db_unread(&self, kind: &str, sub: Option<&str>) -> i64 {
        let conn = self.d.db().reader().expect("reader");
        let n = match sub {
            Some(s) => conn.query_row(
                "SELECT count(*) FROM events WHERE unread = 1 AND kind = ?1 AND sub = ?2",
                [kind, s],
                |r| r.get(0),
            ),
            None => conn.query_row(
                "SELECT count(*) FROM events WHERE unread = 1 AND kind = ?1",
                [kind],
                |r| r.get(0),
            ),
        };
        n.unwrap_or(-1)
    }

    pub fn add(&self, kind: &str, sub: &str, text: &str, session: Option<i64>) -> i64 {
        let e = fake_agentd::events_db::NewEvent {
            kind,
            sub,
            text,
            session_id: session,
            source: Some("test"),
            ts: self.d.now_unix(),
            ..Default::default()
        };
        match self.d.add_event(&e) {
            Ok(Some(id)) => id,
            other => panic!("event not added: {other:?}"),
        }
    }

    pub fn ids(&self) -> Vec<i64> {
        self.n.view().cards.iter().map(|c| c.id).collect()
    }

    pub async fn first_session(&self) -> i64 {
        let list = self.http.list_sessions().await.unwrap_or_default();
        list.iter()
            .find(|s| s["parent_id"].is_null())
            .and_then(|s| s["id"].as_i64())
            .unwrap_or(-1)
    }

    pub fn value(&self, v: &Value, key: &str) -> i64 {
        v[key].as_i64().unwrap_or(-1)
    }
}
