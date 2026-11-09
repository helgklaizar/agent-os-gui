//! Shared stand of the session tree tests against `fake-agentd`: `Rig`, waiting for events, effect execution.
#![allow(dead_code)]

use std::time::Instant;

use crate::common::{WAIT, client, http, start};
use controllers::bar_cluster::BarCluster;
use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::timezone::Zone;
use controllers::tree_types::NodeId;
use controllers::{BusEvent, Controller, Effect};
use core_client::{ConnState, CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use serde_json::Value;
use ui::tree_exec::execute;
use ui::tree_feed::{TOPICS, apply_personas, route};

pub struct Rig {
    pub d: FakeAgentd,
    pub _c: CoreClient,
    pub http: HttpClient,
    pub tree: SessionTreeController,
    pub dialog: DialogController,
    pub bar: BarCluster,
    pub rx: async_channel::Receiver<CoreEvent>,
}

pub async fn rig() -> Rig {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    for t in TOPICS {
        assert!(c.subscribe(t).is_ok());
    }
    let http = http(&d, Some(d.admin_token()));
    let mut r = Rig {
        d,
        _c: c,
        http,
        tree: SessionTreeController::new(),
        dialog: DialogController::new(),
        bar: BarCluster::new(Zone::fixed(0)),
        rx,
    };
    r.load_personas().await;
    r.pump("sessions snapshot", |t| !t.view().rows.is_empty())
        .await;
    r
}

pub fn s(id: i64) -> NodeId {
    NodeId::Session(id)
}

pub fn f(id: i64) -> NodeId {
    NodeId::Folder(id)
}

impl Rig {
    pub async fn load_personas(&mut self) {
        let list = self.http.list_personas().await.unwrap_or_default();
        apply_personas(&mut self.tree, &list, Instant::now());
    }

    pub async fn pump(&mut self, what: &str, done: impl Fn(&SessionTreeController) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(&self.tree) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!(
                    "timed out waiting for: {what}; rows: {:?}",
                    self.tree
                        .view()
                        .rows
                        .iter()
                        .map(|r| (r.node, r.name.clone()))
                        .collect::<Vec<_>>()
                )
            };
            self.feed(&ev);
        }
    }

    pub fn feed(&mut self, ev: &CoreEvent) {
        let now = Instant::now();
        route(&mut self.tree, ev, now);
        if let CoreEvent::State(st) = ev {
            self.bar.on_connection(map_conn(*st), now);
        }
    }

    /// Executes effects: bus -> dialog, command -> core request. Returns command results.
    pub async fn run(&mut self, effects: Vec<Effect>) -> Vec<Result<(), String>> {
        let mut pending = effects;
        let mut results = Vec::new();
        while let Some(e) = pending.pop() {
            match e {
                Effect::Publish(BusEvent::OpenForm { form, opener }) => {
                    pending.extend(
                        self.dialog
                            .update(DialogMsg::OpenForm { form, opener }, Instant::now()),
                    );
                }
                Effect::Command(cmd) => {
                    results.push(
                        execute(&self.http, &cmd)
                            .await
                            .unwrap_or_else(|| Err(format!("command not executed: {cmd:?}"))),
                    );
                }
                _ => {}
            }
        }
        results
    }

    pub async fn tree_msg(&mut self, msg: TreeMsg) -> Vec<Result<(), String>> {
        let fx = self.tree.update(msg, Instant::now());
        self.run(fx).await
    }

    pub async fn dlg(&mut self, msg: DialogMsg) -> Vec<Result<(), String>> {
        let fx = self.dialog.update(msg, Instant::now());
        self.run(fx).await
    }

    pub fn has(&self, node: NodeId) -> bool {
        self.tree.view().rows.iter().any(|r| r.node == node)
    }

    pub fn name_node(&self, name: &str) -> Option<NodeId> {
        self.tree
            .view()
            .rows
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.node)
    }

    pub fn db_ids(&self, table: &str) -> Vec<i64> {
        let conn = self.d.db().reader().expect("reader");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id FROM {table} WHERE archived = 0 OR 1 = 1 ORDER BY id"
            ))
            .or_else(|_| conn.prepare(&format!("SELECT id FROM {table} ORDER BY id")))
            .expect("query");
        stmt.query_map([], |r| r.get(0))
            .expect("rows")
            .flatten()
            .collect()
    }
}

fn map_conn(st: ConnState) -> controllers::ConnState {
    match st {
        ConnState::Connecting => controllers::ConnState::Connecting,
        ConnState::Connected => controllers::ConnState::Connected,
        ConnState::Reconnecting => controllers::ConnState::Reconnecting,
        ConnState::Offline => controllers::ConnState::Offline,
        ConnState::CoreUnavailable => controllers::ConnState::CoreUnavailable,
    }
}

pub fn session_id(v: &Value) -> i64 {
    v["id"].as_i64().unwrap_or(-1)
}

impl Rig {
    /// Parses events until the condition over the whole stand holds.
    pub async fn until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(self) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!("timed out waiting for: {what}")
            };
            self.feed(&ev);
        }
    }

    pub async fn pump_state(&mut self, want: ConnState) {
        let end = Instant::now() + WAIT;
        let mut seen = false;
        while !seen {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!("timed out waiting for state {want:?}")
            };
            seen = ev == CoreEvent::State(want);
            self.feed(&ev);
        }
    }
}
