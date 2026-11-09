//! Core document fields per Settings page: what is editable on a page.

use crate::i18n::tr;
use crate::settings_fields::{Spec, Ty, ai_options, opts, persona_options, spec};
use crate::settings_model::{CoreModel, id_of};
use crate::types::SettingsPage;

/// No `Yes / No` choice list is needed for switches: a switch is a separate type.
fn account_specs() -> Vec<Spec> {
    vec![
        spec("account", "name", tr("set-f-name"), Ty::Text(80), ""),
        spec(
            "account",
            "language",
            tr("set-f-language"),
            Ty::Choice(opts(&[("en", tr("set-lang-en"))])),
            "en",
        ),
    ]
}

fn balance_specs(m: &CoreModel) -> Vec<Spec> {
    let d = "personas";
    vec![
        spec(
            d,
            "balance.fallback",
            tr("set-f-fallback"),
            Ty::Choice(opts(&[
                ("local", tr("set-fb-local")),
                ("other", tr("set-fb-other")),
                ("wait", tr("set-fb-wait")),
            ])),
            "local",
        ),
        spec(d, "balance.warn_pct", tr("set-f-warn"), Ty::Int(1, 100), ""),
        spec(
            d,
            "balance.priority_persona_id",
            tr("set-f-priority"),
            Ty::ChoiceId(persona_options(m)),
            "",
        ),
        spec(
            d,
            "balance.distribution",
            tr("set-f-distribution"),
            Ty::Choice(opts(&[
                ("even", tr("set-dist-even")),
                ("priority", tr("set-dist-priority")),
                ("cheapest", tr("set-dist-cheapest")),
            ])),
            "even",
        ),
        spec(
            d,
            "balance.max_parallel",
            tr("set-f-parallel"),
            Ty::Int(1, 16),
            "",
        ),
        spec(d, "balance.unified_feed", tr("set-f-feed"), Ty::Toggle, ""),
        spec(
            d,
            "balance.shared_memory",
            tr("set-f-shared-mem"),
            Ty::Toggle,
            "",
        ),
        spec(
            d,
            "balance.quiet_from",
            tr("set-f-quiet-from"),
            Ty::Time,
            "",
        ),
        spec(d, "balance.quiet_to", tr("set-f-quiet-to"), Ty::Time, ""),
    ]
}

fn role_specs(m: &CoreModel) -> Vec<Spec> {
    let d = "personas";
    vec![
        spec(
            d,
            "roles.chat",
            tr("set-role-chat"),
            Ty::ChoiceId(ai_options(m)),
            "",
        ),
        spec(
            d,
            "roles.observer",
            tr("set-role-observer"),
            Ty::ChoiceId(ai_options(m)),
            "",
        ),
        spec(
            d,
            "roles.code",
            tr("set-role-code"),
            Ty::ChoiceId(ai_options(m)),
            "",
        ),
        spec(
            d,
            "roles.feed",
            tr("set-role-feed"),
            Ty::ChoiceId(ai_options(m)),
            "",
        ),
    ]
}

/// Cloud account fields on the "Models" page.
pub fn provider_specs(account_id: i64) -> Vec<Spec> {
    let base = format!("accounts.{account_id}");
    vec![
        spec(
            "models",
            &format!("{base}.model"),
            tr("set-f-model"),
            Ty::Text(200),
            "",
        ),
        spec(
            "models",
            &format!("{base}.base_url"),
            tr("set-f-base-url"),
            Ty::Text(2048),
            "",
        ),
    ]
}

fn security_specs() -> Vec<Spec> {
    vec![
        spec(
            "account",
            "security.observer",
            tr("set-f-observer"),
            Ty::Toggle,
            "",
        ),
        spec(
            "account",
            "security.telemetry",
            tr("set-f-telemetry"),
            Ty::Toggle,
            "",
        ),
        spec(
            "account",
            "security.injection_guard",
            tr("set-f-guard"),
            Ty::Toggle,
            "",
        ),
        spec(
            "memory",
            "source",
            tr("set-f-memory-source"),
            Ty::Choice(opts(&[
                ("local", tr("set-mem-local")),
                ("mcp_memory", tr("set-mem-mcp")),
                ("both", tr("set-mem-both")),
            ])),
            "local",
        ),
        spec(
            "schedule",
            "backup.enabled",
            tr("set-f-backup"),
            Ty::Toggle,
            "",
        ),
    ]
}

fn schedule_specs() -> Vec<Spec> {
    let d = "schedule";
    vec![
        spec(d, "summary.enabled", tr("set-f-summary"), Ty::Toggle, ""),
        spec(d, "summary.time", tr("set-f-summary-time"), Ty::Time, ""),
        spec(d, "backup.enabled", tr("set-f-backup"), Ty::Toggle, ""),
        spec(d, "backup.time", tr("set-f-backup-time"), Ty::Time, ""),
        spec(
            d,
            "backup.recipient.primary",
            tr("set-f-recipient"),
            Ty::Text(512),
            "",
        ),
        spec(
            d,
            "backup.recipient.recovery",
            tr("set-f-recovery"),
            Ty::Text(512),
            "",
        ),
    ]
}

/// Document fields of a page (lists and record actions are built by `settings_pages`).
pub fn page_specs(page: SettingsPage, m: &CoreModel) -> Vec<Spec> {
    match page {
        SettingsPage::Account => account_specs(),
        SettingsPage::Personas => balance_specs(m),
        SettingsPage::Models => {
            let mut v = vec![spec(
                "models",
                "context_budget",
                tr("set-f-context"),
                Ty::Int(256, 4_000_000),
                "",
            )];
            for a in m.ai_accounts() {
                if let Some(id) = id_of(a) {
                    v.extend(provider_specs(id));
                }
            }
            v.extend(role_specs(m));
            v
        }
        SettingsPage::Security => security_specs(),
        SettingsPage::Schedule => schedule_specs(),
        SettingsPage::Permissions => vec![spec(
            "permissions",
            "ask_ttl_min",
            tr("set-f-ask-ttl"),
            Ty::Int(1, 1440),
            "",
        )],
        SettingsPage::Tunnels => vec![spec(
            "tunnels",
            "api_external",
            tr("set-f-api-external"),
            Ty::Toggle,
            "",
        )],
        _ => Vec::new(),
    }
}
