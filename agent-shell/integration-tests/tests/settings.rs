//! Every Settings field survives a restart of the core and the GUI; pages show one state.

mod settings_common;

use controllers::SettingsPage;
use controllers::bar_types::parse_quota_snapshot;
use controllers::settings_fields::{Ty, page_specs};
use controllers::settings_view::{Row, SettingsMsg};
use settings_common::*;

const PAGES: [SettingsPage; 7] = [
    SettingsPage::Account,
    SettingsPage::Personas,
    SettingsPage::Models,
    SettingsPage::Security,
    SettingsPage::Schedule,
    SettingsPage::Permissions,
    SettingsPage::Tunnels,
];

#[tokio::test(flavor = "multi_thread")]
async fn every_settings_field_persists_across_restart() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Account).await;
    let mut expected: Vec<(SettingsPage, String, String)> = Vec::new();
    for page in PAGES {
        r.select(page).await;
        let specs = page_specs(page, &r.c_model());
        assert!(!specs.is_empty(), "{page:?}: the page has fields");
        for s in &specs {
            let current = r.field(&s.id).map(|f| f.value).unwrap_or_default();
            let new = sample(s, &current);
            if new.is_empty() {
                // a choice with a single option (the language list is English only) has no other value to check
                assert!(
                    matches!(&s.ty, Ty::Choice(o) if o.len() == 1),
                    "{}: no value to check",
                    s.id
                );
                continue;
            }
            r.edit(&s.id, &new).await;
            assert!(
                r.field(&s.id).is_some_and(|f| f.error.is_none()),
                "{}: {new:?} rejected before sending",
                s.id
            );
            // one field on two pages (backup: "Security" and "Schedule") is one value: the last write wins
            expected.retain(|(_, id, _)| *id != s.id);
            expected.push((page, s.id.clone(), new));
        }
        r.save().await;
        assert!(
            !r.view().dirty && !r.view().saving,
            "{page:?}: saved without errors: {:?}",
            r.banner()
        );
    }
    // the global permission, a secret and indicators are separate write paths
    r.select(SettingsPage::Permissions).await;
    r.edit("perm:global:read", "ask").await;
    r.save().await;
    r.select(SettingsPage::Indicators).await;
    r.act(controllers::settings_types::Action::IndicatorDown(
        "cpu".into(),
    ))
    .await;
    r.save().await;
    let order_before = r.titles();
    let mut r = r.restart().await;
    r.open(SettingsPage::Account).await;
    for (page, id, new) in expected {
        r.select(page).await;
        let got = r.field(&id).map(|f| f.value);
        assert_eq!(got.as_deref(), Some(new.as_str()), "{id} after a restart");
    }
    r.select(SettingsPage::Permissions).await;
    assert_eq!(
        r.field("perm:global:read").map(|f| f.value).as_deref(),
        Some("ask")
    );
    r.select(SettingsPage::Indicators).await;
    assert_eq!(r.titles(), order_before, "indicator order saved");
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_pages_consistent_on_one_fixture() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Connections).await;
    let accounts = r.http.list_accounts().await.expect("accounts");
    assert!(accounts.len() >= 2, "the fixture gives accounts");
    let labels: Vec<String> = accounts.iter().map(|a| value(&a["label"])).collect();
    // "Connections": all accounts
    let connections = r.titles();
    for l in &labels {
        assert!(
            connections.contains(l),
            "'Connections' without {l}: {connections:?}"
        );
    }
    // "Models": AI accounts from the same list, the local model load matches the bar frame
    let quotas = parse_quota_snapshot(&r.d.snapshot("bar.quota")).expect("quota snapshot");
    r.msg(SettingsMsg::Quotas(quotas.clone())).await;
    r.select(SettingsPage::Models).await;
    let models = r.titles();
    for q in quotas
        .iter()
        .filter(|q| q.kind == controllers::bar_types::QuotaKind::ApiKey)
    {
        assert!(
            labels.contains(&q.label),
            "quota {} is among the accounts",
            q.label
        );
    }
    let text: String = r
        .view()
        .rows
        .iter()
        .filter_map(|x| {
            if let Row::Item(i) = x {
                Some(i.subtitle.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    for q in quotas
        .iter()
        .filter(|q| q.kind == controllers::bar_types::QuotaKind::Local)
    {
        let load = q
            .load
            .map_or_else(String::new, |l| format!("{:.0}%", l * 100.0));
        assert!(
            models.contains(&q.label),
            "local model {} on the page",
            q.label
        );
        assert!(text.contains(&load), "load {load} matches the bar: {text}");
    }
    // "Personas": the same accounts in the switches
    r.select(SettingsPage::Personas).await;
    let persona_fields: Vec<String> = r
        .view()
        .rows
        .iter()
        .filter_map(|x| {
            if let Row::Field(f) = x {
                Some(f.label.clone())
            } else {
                None
            }
        })
        .collect();
    for l in &labels {
        assert!(persona_fields.contains(l), "'Personas' without account {l}");
    }
    // an account added from outside is visible everywhere after re-reading, without a restart
    r.http
        .create_account(serde_json::json!({"type": "tg", "label": "Fresh"}))
        .await
        .expect("creation");
    r.msg(SettingsMsg::Open(SettingsPage::Personas)).await;
    r.msg(SettingsMsg::Changed {
        page: "personas".into(),
        rev: 999,
    })
    .await;
    r.msg(SettingsMsg::Reply(
        controllers::settings_types::Reply::Accounts(
            r.http.list_accounts().await.expect("accounts"),
        ),
    ))
    .await;
    let after: Vec<String> = r
        .view()
        .rows
        .iter()
        .filter_map(|x| {
            if let Row::Field(f) = x {
                Some(f.label.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(after.contains(&"Fresh".to_string()));
    r.select(SettingsPage::Connections).await;
    assert!(r.titles().contains(&"Fresh".to_string()));
}
