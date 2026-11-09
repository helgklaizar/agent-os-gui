//! Modules against `fake-agentd` over a real socket. An installed module appears as a panel,
//! widget or button from `modules` topic frames, enabling and disabling work without a restart, a bad manifest
//! and a module crash do not break the shell.

mod common;

use std::time::Instant;

use common::{WAIT, client, http, start};
use controllers::Controller;
use controllers::controllers::modules::{ModulesController, ModulesMsg};
use core_client::{CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use serde_json::json;
use ui::modules_feed::{self, apply_list};

struct Rig {
    d: FakeAgentd,
    c: CoreClient,
    http: HttpClient,
    m: ModulesController,
    rx: async_channel::Receiver<CoreEvent>,
}

async fn rig() -> Rig {
    let d = start(Config::temp().no_ticker()).await;
    let c = client(&d);
    let rx = c.events();
    for t in modules_feed::TOPICS {
        assert!(c.subscribe(t).is_ok(), "subscription {t}");
    }
    let http = http(&d, Some(d.admin_token()));
    Rig {
        d,
        c,
        http,
        m: ModulesController::default(),
        rx,
    }
}

impl Rig {
    async fn reload(&mut self) {
        let list = self.http.list_modules().await.expect("GET /modules");
        apply_list(&mut self.m, &list, Instant::now());
    }

    /// Parses core client events until the condition holds; the list is re-read when the bridge asks.
    async fn until(&mut self, what: &str, done: impl Fn(&ModulesController) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(&self.m) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!("timed out waiting for: {what}; view: {:?}", self.m.view());
            };
            if modules_feed::route(&mut self.m, &ev, Instant::now()).refresh {
                self.reload().await;
            }
        }
    }

    async fn install(&self, catalog_id: &str) {
        self.http
            .install_module(json!({ "catalog_id": catalog_id }))
            .await
            .unwrap_or_else(|e| panic!("install {catalog_id}: {e:?}"));
    }

    async fn set_enabled(&self, id: &str, on: bool) {
        self.http
            .patch_module(id, json!({ "enabled": on }))
            .await
            .unwrap_or_else(|e| panic!("PATCH {id}: {e:?}"));
    }
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    items.iter().map(|i| id(i).to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_render_by_manifest_from_live_core() {
    let mut r = rig().await;
    for id in ["weather-card", "notes-panel", "quick-btn"] {
        r.install(id).await;
    }
    r.until("three placements", |m| {
        let v = m.view();
        v.panels.len() == 1 && v.widgets.len() == 1 && v.buttons.len() == 1
    })
    .await;
    let v = r.m.view();
    assert_eq!(ids(&v.panels, |p| &p.id), ["notes-panel"]);
    assert_eq!(ids(&v.widgets, |w| &w.id), ["weather-card"]);
    assert_eq!(ids(&v.buttons, |b| &b.id), ["quick-btn"]);
    assert_eq!(
        v.widgets[0].title, "Weather",
        "title from the core manifest"
    );
    assert!(v.rejected.is_empty(), "{:?}", v.rejected);
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_toggle_and_remove_without_restart() {
    let mut r = rig().await;
    r.install("weather-card").await;
    r.install("quick-btn").await;
    r.until("widget and button", |m| {
        let v = m.view();
        v.widgets.len() == 1 && v.buttons.len() == 1
    })
    .await;
    r.set_enabled("weather-card", false).await;
    r.until("widget gone", |m| m.view().widgets.is_empty())
        .await;
    assert_eq!(r.m.view().buttons.len(), 1, "the button is untouched");
    r.set_enabled("weather-card", true).await;
    r.until("widget returned", |m| m.view().widgets.len() == 1)
        .await;
    r.http.delete_module("quick-btn").await.expect("removal");
    r.until("button removed", |m| m.view().buttons.is_empty())
        .await;
    assert_eq!(r.m.view().widgets.len(), 1);
    drop((r.c, r.d));
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_state_survives_reconnect_via_list() {
    let mut r = rig().await;
    r.install("notes-panel").await;
    r.until("panel", |m| m.view().panels.len() == 1).await;
    // a new controller, as after a shell restart: state is taken from GET /modules
    r.m = ModulesController::default();
    r.reload().await;
    assert_eq!(ids(&r.m.view().panels, |p| &p.id), ["notes-panel"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_bad_manifest_from_core_does_not_break_shell() {
    let mut r = rig().await;
    r.install("notes-panel").await;
    r.install("quick-btn").await;
    // a row with an unsuitable manifest (a write bypassing the core check: an old version, a manual DB edit)
    r.d.db()
        .write(|c| {
            c.execute(
                "INSERT INTO modules(id, name, manifest_json, installed, enabled, placement, config_json) \
                 VALUES ('bad-one', 'Bad', '{\"id\":\"bad-one\",\"sneaky\":1}', 1, 1, 'widget', '{}')",
                [],
            )
        })
        .expect("write");
    r.reload().await;
    let v = r.m.view();
    assert_eq!(ids(&v.rejected, |x| &x.id), ["bad-one"]);
    assert!(
        v.rejected[0].reason.contains("sneaky"),
        "the error names the field: {}",
        v.rejected[0].reason
    );
    assert_eq!(v.panels.len(), 1, "good modules in place");
    assert_eq!(v.buttons.len(), 1);
    assert!(v.widgets.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_crash_of_one_panel_does_not_touch_others() {
    let mut r = rig().await;
    r.install("notes-panel").await;
    r.install("weather-card").await;
    r.until("panel and widget", |m| {
        let v = m.view();
        v.panels.len() == 1 && v.widgets.len() == 1
    })
    .await;
    r.m.update(
        ModulesMsg::SelectPanel("notes-panel".into()),
        Instant::now(),
    );
    r.m.update(ModulesMsg::Crashed("notes-panel".into()), Instant::now());
    let v = r.m.view();
    assert!(v.panels[0].crashed);
    assert_eq!(v.widgets.len(), 1);
    // the live feed keeps working after a panel failure
    r.set_enabled("weather-card", false).await;
    r.until("feed alive", |m| m.view().widgets.is_empty()).await;
    assert!(
        r.m.view().panels[0].crashed,
        "the panel crash is remembered"
    );
}
