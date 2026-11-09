//! Common rig for gate tests: real stores on a temporary DB, event capture.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use agentd_core::{
    Action, Actor, Audit, Gate, GateInput, Level, ManualClock, Mode, Notifier, Observer,
    PermEngine, PermissionService, PolicyGate, Rule, RuleObserver, RuleStore, Subject,
};
use agentd_db::{ChatRepo, Db, DbConfig, PermRepo, SettingsRepo};
use serde_json::{Value, json};

#[derive(Default)]
pub struct Events(pub Mutex<Vec<(String, Value)>>);

impl Notifier for Events {
    fn publish(&self, topic: &str, data: Value) {
        self.0.lock().unwrap().push((topic.to_string(), data));
    }
}

pub struct Rig {
    pub _dir: tempfile::TempDir,
    pub db: Arc<Db>,
    pub gate: PolicyGate,
    pub engine: PermEngine,
    pub chat: Arc<ChatRepo>,
    pub observer: Arc<RuleObserver>,
    pub events: Arc<Events>,
}

pub async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1);
             INSERT INTO personas(id, name, color) VALUES (7,'p','#fff');",
        )
    })
    .await
    .unwrap();
    let clock = Arc::new(ManualClock::new(1_800_000_000));
    let repo = Arc::new(PermRepo::new(db.clone()));
    repo.seed_defaults().await.unwrap();
    let audit = Audit::new(repo.clone(), clock.clone());
    let engine = PermEngine::new(repo.clone(), audit.clone());
    let chat = Arc::new(ChatRepo::new(
        db.clone(),
        clock.clone(),
        dir.path().join("agentos"),
    ));
    let events = Arc::new(Events::default());
    let settings = Arc::new(SettingsRepo::new(db.clone(), clock.clone()));
    let requests = PermissionService::new(
        audit.clone(),
        chat.clone(),
        events.clone(),
        clock,
        settings,
        engine.clone(),
    );
    let observer = Arc::new(RuleObserver::load(None).unwrap());
    let obs: Arc<dyn Observer> = observer.clone();
    let gate = PolicyGate::new(engine.clone(), obs, audit, requests);
    Rig {
        _dir: dir,
        db,
        gate,
        engine,
        chat,
        observer,
        events,
    }
}

pub fn input(action: Action, target: &str) -> GateInput {
    GateInput {
        subject: Subject {
            actor: Actor::Agent,
            session_id: Some(1),
            persona_id: Some(7),
        },
        action,
        target: Some(target.into()),
        args: Some(json!({"token": "tok-VALUE-1", "x": 1})),
        untrusted: None,
        grant_token: None,
    }
}

pub async fn set(r: &Rig, level: Level, id: i64, action: Action, mode: Mode) {
    r.engine
        .put(
            Rule {
                level,
                scope_id: id,
                action,
                mode,
            },
            "test",
        )
        .await
        .unwrap();
}

pub async fn row(r: &Rig, id: i64) -> (Option<String>, Option<String>, Option<String>) {
    r.db.read(move |c| {
        c.query_row(
            "SELECT decision, result, args_json FROM actions_log WHERE id = ?1",
            [id],
            |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)),
        )
    })
    .await
    .unwrap()
}

pub async fn findings(r: &Rig) -> Vec<(String, String, String, Option<String>)> {
    r.db.read(|c| {
        c.prepare("SELECT kind, verdict, ref_table, ref_id FROM observer_findings ORDER BY id")?
            .query_map([], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)))?
            .collect()
    })
    .await
    .unwrap()
}

pub fn ask(g: Gate) -> agentd_core::PermissionRequest {
    match g {
        Gate::Ask(r) => *r,
        other => panic!("expected Ask: {other:?}"),
    }
}
