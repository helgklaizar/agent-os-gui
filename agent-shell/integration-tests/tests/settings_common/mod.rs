//! Shared by the Settings integration tests: `SettingsController` against `fake-agentd`. Controller commands
//! are executed by the same `ui::settings_exec` as the GUI worker thread; replies go back to the controller as in the product build.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::Instant;

use controllers::controllers::settings::SettingsController;
use controllers::settings_fields::{Spec, Ty};
use controllers::settings_types::Action;
use controllers::settings_view::{FieldRow, Row, SettingsMsg, SettingsView};
use controllers::{Controller, CoreCommand, Effect, SettingsPage};
use core_client::{ClientError, HttpClient, Token};
use fake_agentd::{Config, FakeAgentd};
use serde_json::Value;

pub struct Rig {
    pub d: FakeAgentd,
    pub http: HttpClient,
    pub c: SettingsController,
    pub dir: tempfile::TempDir,
    /// All effects the controller returned (for "nothing went out" checks).
    pub effects: Vec<Effect>,
}

fn client(d: &FakeAgentd) -> HttpClient {
    HttpClient::new(
        core_client::Endpoint::Unix(d.socket_path().to_path_buf()),
        Some(Token::new(d.admin_token())),
    )
    .with_timeout(std::time::Duration::from_secs(5))
}

async fn fake(dir: &std::path::Path) -> FakeAgentd {
    match FakeAgentd::start(Config::temp().no_ticker().db(dir.join("sessions.db"))).await {
        Ok(d) => d,
        Err(e) => panic!("fake-agentd did not start: {e}"),
    }
}

impl Rig {
    pub async fn start() -> Rig {
        let dir = tempfile::tempdir().expect("data directory");
        let d = fake(dir.path()).await;
        Rig {
            http: client(&d),
            d,
            c: SettingsController::new(),
            dir,
            effects: Vec::new(),
        }
    }

    /// Restart of the core and the GUI: the same DB file, a new mock process, a new controller.
    pub async fn restart(self) -> Rig {
        let Rig { d, dir, .. } = self;
        drop(d);
        let d = fake(dir.path()).await;
        Rig {
            http: client(&d),
            d,
            c: SettingsController::new(),
            dir,
            effects: Vec::new(),
        }
    }

    /// A message to the controller; commands are executed and replies returned until no commands remain.
    pub async fn msg(&mut self, m: SettingsMsg) {
        let mut queue = VecDeque::from([m]);
        while let Some(m) = queue.pop_front() {
            let fx = self.c.update(m, Instant::now());
            for e in &fx {
                if let Effect::Command(CoreCommand::Settings(cmd)) = e {
                    for r in ui::settings_exec::execute(&self.http, cmd).await {
                        queue.push_back(SettingsMsg::Reply(r));
                    }
                }
            }
            self.effects.extend(fx);
        }
    }

    pub async fn open(&mut self, page: SettingsPage) {
        self.msg(SettingsMsg::Open(page)).await;
    }

    pub async fn select(&mut self, page: SettingsPage) {
        self.msg(SettingsMsg::Select(page)).await;
    }

    pub async fn edit(&mut self, id: &str, value: &str) {
        self.msg(SettingsMsg::Edit {
            id: id.into(),
            value: value.into(),
        })
        .await;
    }

    pub async fn save(&mut self) {
        self.msg(SettingsMsg::Save).await;
    }

    pub async fn act(&mut self, a: Action) {
        self.msg(SettingsMsg::Act(a)).await;
    }

    pub fn c_model(&self) -> controllers::settings_model::CoreModel {
        self.c.model().clone()
    }

    pub fn view(&self) -> SettingsView {
        self.c.view()
    }

    pub fn field(&self, id: &str) -> Option<FieldRow> {
        self.view().rows.into_iter().find_map(|r| match r {
            Row::Field(f) if f.id == id => Some(f),
            _ => None,
        })
    }

    pub fn titles(&self) -> Vec<String> {
        self.view()
            .rows
            .into_iter()
            .filter_map(|r| {
                if let Row::Item(i) = r {
                    Some(i.title)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn banner(&self) -> Option<String> {
        self.view().banner.map(|b| b.text)
    }
}

/// A new value for a field, different from the current one and valid for the core.
pub fn sample(spec: &Spec, current: &str) -> String {
    match &spec.ty {
        Ty::Text(_) if spec.path.ends_with("base_url") => "https://api.example.test/v9".into(),
        Ty::Text(_) => format!("new-{}", spec.path.replace('.', "-")),
        Ty::Toggle => (current != "true").to_string(),
        Ty::Choice(o) | Ty::ChoiceId(o) => o
            .iter()
            .find(|(v, _)| v != current && !v.is_empty())
            .map(|(v, _)| v.clone())
            .unwrap_or_default(),
        Ty::Int(lo, hi) => {
            let n: i64 = current.parse().unwrap_or(*lo);
            if n < *hi { n + 1 } else { n - 1 }.to_string()
        }
        Ty::Time => if current == "05:45" { "06:15" } else { "05:45" }.into(),
    }
}

pub fn value(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// HTTP status of an API error from the client response.
pub fn status(e: &Result<impl std::fmt::Debug, ClientError>) -> Option<u16> {
    e.as_ref().err().and_then(|e| e.api()).map(|a| a.status)
}
