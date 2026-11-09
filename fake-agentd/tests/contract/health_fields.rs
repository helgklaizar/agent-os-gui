//! Fields of `GET /v1/health` (api.md section 2.1) in the mock in all core modes.

use fake_agentd::CoreMode;
use serde_json::Value;

use crate::openapi::Spec;
use crate::rawhttp::{Req, send};
use crate::{assert_clean, mock};

async fn health(d: &fake_agentd::FakeAgentd) -> Value {
    let r = send(d.socket_path(), Req::new("GET", "/v1/health")).await;
    assert_eq!(r.status, 200, "health is always 200");
    r.json()
}

#[test]
fn contract_health_schema_requires_every_documented_field() {
    let spec = Spec::load();
    let required = spec.schema("Health").get("required").map(|r| r.strs());
    let required = required.unwrap_or_default();
    for k in [
        "status",
        "version",
        "api",
        "min_client",
        "min_gui",
        "max_gui",
        "user_version",
        "capabilities",
        "migration",
    ] {
        assert!(
            required.contains(&k),
            "Health.required without {k}: {required:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_health_has_all_fields_in_every_mode() {
    let d = mock().await;
    let spec = Spec::load();
    for (mode, status) in [
        (CoreMode::Ok, "ok"),
        (CoreMode::Down, "starting"),
        (CoreMode::Migrating, "migrating"),
    ] {
        d.set_mode(mode);
        let h = health(&d).await;
        assert_eq!(h["status"], status);
        assert_clean(
            "GET /health",
            spec.validate(spec.schema("Health"), &h, "Health"),
        );
        for k in ["min_client", "min_gui", "max_gui"] {
            assert!(h[k].as_str().is_some_and(|s| !s.is_empty()), "{k}: {h}");
        }
        assert!(
            h["capabilities"].as_array().is_some_and(|c| !c.is_empty()),
            "{h}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_health_user_version_is_the_database_user_version() {
    let d = mock().await;
    let h = health(&d).await;
    let conn = d.db().reader().expect("DB reader");
    let uv: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version");
    assert!(uv > 0, "migrations applied");
    assert_eq!(h["user_version"], uv, "{h}");
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_health_migration_is_null_when_ok_and_object_when_migrating() {
    let d = mock().await;
    let spec = Spec::load();
    assert_eq!(health(&d).await["migration"], Value::Null);
    d.set_mode(CoreMode::Migrating);
    let m = health(&d).await["migration"].clone();
    assert!(m.is_object(), "{m}");
    assert_clean(
        "health.migration",
        spec.validate(spec.schema("Migration"), &m, "Migration"),
    );
    assert!(
        m["from"].as_i64() < m["to"].as_i64(),
        "from < to during a migration: {m}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_health_gui_range_is_ordered() {
    let d = mock().await;
    let h = health(&d).await;
    let parse =
        |s: &str| -> Vec<u64> { s.split('.').map_while(|p| p.parse::<u64>().ok()).collect() };
    let (lo, hi) = (
        parse(h["min_gui"].as_str().unwrap_or("")),
        parse(h["max_gui"].as_str().unwrap_or("")),
    );
    assert!(!lo.is_empty(), "min_gui is a version: {h}");
    // `max_gui` allows a wildcard (`0.x`): compare by numeric parts up to the wildcard
    assert!(!hi.is_empty(), "max_gui is a version or a wildcard: {h}");
    assert!(lo[0] <= hi[0], "min_gui is not above max_gui: {h}");
    assert!(
        h["min_client"]
            .as_str()
            .is_some_and(|s| !parse(s).is_empty()),
        "min_client is a version: {h}"
    );
}
