//! Settings against a real `agentd`: the same Settings pages are read and saved through the controller and the core client
//! (routes `GET`/`PATCH /v1/settings/{page}` with `If-Match`, personas, accounts, indicators).
mod real_rig;

use std::collections::VecDeque;
use std::time::Instant;

use controllers::controllers::settings::SettingsController;
use controllers::settings_fields::page_specs;
use controllers::settings_types::Action;
use controllers::settings_view::{Row, SettingsMsg};
use controllers::{Controller, CoreCommand, Effect, SettingsPage};
use core_client::HttpClient;
use real_rig::Real;
use serde_json::json;

async fn msg(c: &mut SettingsController, http: &HttpClient, m: SettingsMsg) {
    let mut queue = VecDeque::from([m]);
    while let Some(m) = queue.pop_front() {
        for e in c.update(m, Instant::now()) {
            if let Effect::Command(CoreCommand::Settings(cmd)) = e {
                for r in ui::settings_exec::execute(http, &cmd).await {
                    queue.push_back(SettingsMsg::Reply(r));
                }
            }
        }
    }
}

fn field(c: &SettingsController, id: &str) -> Option<String> {
    c.view().rows.into_iter().find_map(|r| match r {
        Row::Field(f) if f.id == id => Some(f.value),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn real_settings_document_pages_save_and_reload() {
    let r = Real::start().await;
    let mut c = SettingsController::new();
    msg(&mut c, &r.http, SettingsMsg::Open(SettingsPage::Account)).await;
    assert!(!c.view().loading, "the core answered");
    // do not touch "Tunnels": `api_external` turns on the core's TCP listener
    for page in [
        SettingsPage::Account,
        SettingsPage::Personas,
        SettingsPage::Security,
        SettingsPage::Schedule,
        SettingsPage::Permissions,
    ] {
        msg(&mut c, &r.http, SettingsMsg::Select(page)).await;
        let mut want = Vec::new();
        for s in page_specs(page, c.model()) {
            let current = field(&c, &s.id).unwrap_or_default();
            let new = match &s.ty {
                controllers::settings_fields::Ty::Text(_) => {
                    format!("v-{}", s.path.replace('.', "-"))
                }
                controllers::settings_fields::Ty::Toggle => (current != "true").to_string(),
                controllers::settings_fields::Ty::Choice(o)
                | controllers::settings_fields::Ty::ChoiceId(o) => {
                    match o.iter().find(|(v, _)| *v != current && !v.is_empty()) {
                        Some((v, _)) => v.clone(),
                        None => continue,
                    }
                }
                controllers::settings_fields::Ty::Int(lo, _) => (lo + 1).to_string(),
                controllers::settings_fields::Ty::Time => "05:45".to_string(),
            };
            msg(
                &mut c,
                &r.http,
                SettingsMsg::Edit {
                    id: s.id.clone(),
                    value: new.clone(),
                },
            )
            .await;
            want.push((s.id.clone(), new));
        }
        msg(&mut c, &r.http, SettingsMsg::Save).await;
        let v = c.view();
        assert!(
            !v.dirty && !v.saving,
            "{page:?}: the real core accepted the save: {:?}",
            v.banner
        );
        // a fresh controller (GUI restart) sees the same
        let mut fresh = SettingsController::new();
        msg(&mut fresh, &r.http, SettingsMsg::Open(page)).await;
        for (id, value) in want {
            assert_eq!(
                field(&fresh, &id).as_deref(),
                Some(value.as_str()),
                "{page:?}: {id}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn real_settings_persona_account_indicators_roundtrip() {
    let r = Real::start().await;
    let created = r
        .http
        .create_persona(json!({"name": "Real P", "color": "#112233"}))
        .await
        .expect("persona");
    let pid = created["id"].as_i64().expect("id");
    let mut c = SettingsController::new();
    msg(&mut c, &r.http, SettingsMsg::Open(SettingsPage::Personas)).await;
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Act(Action::PersonaSelect(pid)),
    )
    .await;
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Edit {
            id: format!("p:{pid}:name"),
            value: "Real P2".into(),
        },
    )
    .await;
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Edit {
            id: format!("p:{pid}:lang"),
            value: "en".into(),
        },
    )
    .await;
    msg(&mut c, &r.http, SettingsMsg::Save).await;
    assert!(!c.view().dirty, "{:?}", c.view().banner);
    let list = r.http.list_personas().await.expect("personas");
    let p = list
        .iter()
        .find(|p| p["id"] == pid)
        .expect("persona in the core");
    assert_eq!(
        (p["name"].as_str(), p["lang"].as_str()),
        (Some("Real P2"), Some("en"))
    );
    // a new persona from the form: the default colour goes to the core (it requires a colour)
    msg(&mut c, &r.http, SettingsMsg::Act(Action::PersonaFormNew)).await;
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Edit {
            id: "p:new:name".into(),
            value: "From Form".into(),
        },
    )
    .await;
    msg(&mut c, &r.http, SettingsMsg::Act(Action::PersonaNew)).await;
    assert!(
        c.view().banner.is_none() || !c.view().dirty,
        "{:?}",
        c.view().banner
    );
    let list = r.http.list_personas().await.expect("personas");
    assert!(
        list.iter()
            .any(|p| p["name"] == "From Form" && p["color"] == "#808080"),
        "{list:?}"
    );
    // deletion with confirmation
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Act(Action::PersonaDelete(pid)),
    )
    .await;
    assert!(c.view().confirm.is_some());
    msg(&mut c, &r.http, SettingsMsg::Confirm(true)).await;
    assert!(
        !r.http
            .list_personas()
            .await
            .expect("personas")
            .iter()
            .any(|p| p["id"] == pid)
    );
    // indicators: order and enabling
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Select(SettingsPage::Indicators),
    )
    .await;
    let first = c
        .view()
        .rows
        .iter()
        .find_map(|x| {
            if let Row::Item(i) = x {
                Some(i.title.clone())
            } else {
                None
            }
        })
        .expect("core indicators");
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Act(Action::IndicatorDown(first.clone())),
    )
    .await;
    msg(&mut c, &r.http, SettingsMsg::Save).await;
    assert!(!c.view().dirty, "{:?}", c.view().banner);
    let page = r.http.settings_page("indicators").await.expect("page");
    assert_eq!(
        page.doc["items"][1]["id"],
        first.as_str(),
        "order saved by the real core"
    );
    // a foreign edit: a stale rev gives 412, the input stays
    msg(&mut c, &r.http, SettingsMsg::Select(SettingsPage::Account)).await;
    msg(
        &mut c,
        &r.http,
        SettingsMsg::Edit {
            id: "d:account:name".into(),
            value: "Mine".into(),
        },
    )
    .await;
    let page = r.http.settings_page("account").await.expect("page");
    r.http
        .patch_settings_page("account", page.rev, &json!({"name": "Phone"}))
        .await
        .expect("foreign edit");
    msg(&mut c, &r.http, SettingsMsg::Save).await;
    assert!(
        c.view().banner.is_some() && c.view().dirty,
        "412: the error is visible, the input is intact"
    );
    assert_eq!(field(&c, "d:account:name").as_deref(), Some("Mine"));
    msg(&mut c, &r.http, SettingsMsg::Save).await;
    assert!(
        !c.view().dirty,
        "the retry went through: {:?}",
        c.view().banner
    );
}
