//! Shared test stand for tools against `fake-agentd`: core client, three tab controllers, effect execution.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Instant;

use controllers::{Controller, Effect};
use core_client::{CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use serde_json::{Value, json};
use ui::tools_exec;
use ui::tools_feed::ToolsCtls;

use crate::common::{WAIT, client, http, start};

pub struct Rig {
    pub d: FakeAgentd,
    pub c: CoreClient,
    pub http: HttpClient,
    pub t: ToolsCtls,
    pub rx: async_channel::Receiver<CoreEvent>,
    pub session: i64,
    /// What the executor returned to the controllers and which commands it did not accept.
    pub failures: Vec<String>,
}

pub async fn rig() -> Rig {
    rig_with(Config::temp().no_ticker().no_event_fixtures()).await
}

pub async fn rig_with(config: Config) -> Rig {
    let d = start(config).await;
    let c = client(&d);
    let rx = c.events();
    let http = http(&d, Some(d.admin_token()));
    let made = http
        .create_session("Tools", None, None)
        .await
        .unwrap_or_default();
    let session = made["id"].as_i64().unwrap_or(-1);
    let mut r = Rig {
        d,
        c,
        http,
        t: ToolsCtls::new(PathBuf::from("/home/user")),
        rx,
        session,
        failures: Vec::new(),
    };
    r.attach(session).await;
    r
}

impl Rig {
    /// Subscription to `sessions`, `bar.quota` and the session topic, opening tabs; waits for the `sessions` snapshot.
    pub async fn attach(&mut self, session: i64) {
        for topic in ["sessions", "bar.quota", &format!("session.{session}")] {
            assert!(self.c.subscribe(topic).is_ok(), "subscription {topic}");
        }
        let fx = self.t.open(Some(session), Instant::now());
        self.run(fx).await;
        self.until("sessions snapshot", |r| r.t.settings.view().name == "Tools")
            .await;
    }

    /// Parses client events until the condition holds; effects are executed.
    pub async fn until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(self) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!("timed out waiting for: {what}")
            };
            let fx = self.t.route(&ev, Instant::now());
            self.run(fx).await;
        }
    }

    /// Executes tool commands; core replies go back to the controllers, errors accumulate in `failures`.
    pub async fn run(&mut self, effects: Vec<Effect>) {
        let mut pending: Vec<Effect> = effects.into_iter().rev().collect();
        while let Some(e) = pending.pop() {
            let Effect::Command(cmd) = e else { continue };
            match tools_exec::execute(&self.http, &cmd).await {
                Some(Ok(replies)) => {
                    for reply in replies {
                        let fx = self.t.reply(reply, Instant::now());
                        pending.extend(fx.into_iter().rev());
                    }
                }
                Some(Err(why)) => self.failures.push(why),
                None => match ui::tree_exec::execute(&self.http, &cmd).await {
                    Some(Err(why)) => self.failures.push(why),
                    Some(Ok(())) => {}
                    None => self.failures.push(format!("not executed: {cmd:?}")),
                },
            }
        }
    }

    pub async fn rows(&self, sql: &str) -> Vec<String> {
        let conn = self.d.db().reader().expect("reader");
        let mut st = conn.prepare(sql).expect("query");
        st.query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .flatten()
            .collect()
    }
}

pub fn texts(v: &Value, key: &str) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x[key].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn put_rule_body(mode: &str) -> Value {
    json!({ "mode": mode })
}
