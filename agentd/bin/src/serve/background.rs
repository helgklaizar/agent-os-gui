//! Background tasks of `serve`: `/health` capabilities, hardware metrics, weather, retention.

use std::sync::Arc;

use agentd_core::SettingsStore;

/// Capabilities in `/health` (A7-11): `tool-results` is present while the browser bridge is running; `terminal` while the runner socket
/// exists (the unit is active); `observer-model` when the model-based observer is enabled.
pub(super) fn spawn_capabilities(
    health: agentd_api::HealthControl,
    runner_socket: std::path::PathBuf,
) {
    health.set_capability("tool-results", true);
    health.set_capability("observer-model", crate::watch::enabled());
    health.set_capability("observer-process", crate::watch::socket().is_some());
    tokio::spawn(async move {
        loop {
            health.set_capability("terminal", runner_socket.exists());
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// Hardware metrics every 2 s: measured in a blocking thread, a `delta` frame into `bar.sys`.
pub(super) fn spawn_sys(snap: agentd_core::SysSnapshot, hub: agentd_api::Hub) {
    tokio::spawn(async move {
        let sampler = Arc::new(std::sync::Mutex::new(
            agentd_sys::metrics::Sampler::default(),
        ));
        loop {
            let s = sampler.clone();
            let sample = tokio::task::spawn_blocking(move || {
                s.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .sample()
            })
            .await
            .unwrap_or_default();
            let frame = agentd_core::SysSnapshot::frame(
                sample.cpu,
                sample.gpu,
                sample.ram,
                sample.disk,
                sample.temp,
            );
            snap.set(frame.clone());
            agentd_core::Notifier::publish(&hub, "bar.sys", frame);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

/// Open-Meteo weather every 30 minutes (in a test build the period and address are set by environment variables): coordinates come from the
/// `account` page; no coordinates or an unavailable source gives a frame with `null`.
pub(super) fn spawn_weather(
    snap: agentd_core::WeatherSnapshot,
    settings: Arc<dyn SettingsStore>,
    clock: Arc<dyn agentd_core::Clock>,
    hub: agentd_api::Hub,
) {
    let test = agentd_core::manual_clock(0).is_some();
    let every = std::env::var("AGENTD_WEATHER_EVERY_SECS")
        .ok()
        .filter(|_| test)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30 * 60);
    let url_file = std::env::var_os("AGENTD_WEATHER_URLFILE").filter(|_| test);
    tokio::spawn(async move {
        let http = agentd_providers::HttpClient::new(agentd_providers::Timeouts::default());
        loop {
            let loc = match settings.get("account").await {
                Ok(d) => agentd_core::location_of(&d.doc),
                Err(_) => None,
            };
            let base = url_file
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| agentd_providers::weather::DEFAULT_BASE.to_string());
            let now = match loc {
                Some((lat, lon)) => agentd_providers::weather::fetch(&http, &base, lat, lon).await,
                None => None,
            };
            let frame = agentd_core::WeatherSnapshot::frame(now.as_ref(), clock.now_unix());
            snap.set(frame.clone());
            agentd_core::Notifier::publish(&hub, "bar.weather", frame);
            tokio::time::sleep(std::time::Duration::from_secs(every)).await;
        }
    });
}

/// Retention once a day at 03:30 `Asia/Jerusalem` (assumption A7-08); an error is an `error` event in the feed.
pub(super) fn spawn_retention(
    db: Arc<agentd_db::Db>,
    clock: Arc<dyn agentd_core::Clock>,
    events: agentd_core::Events,
    agentos: &std::path::Path,
) {
    let retention = agentd_db::Retention::new(db)
        .with_snapshots(agentos.join("retention-backups"))
        .with_snapshot_dir(agentos.join("migration-backups"));
    tokio::spawn(async move {
        let next_after = |t: i64| {
            agentd_core::cron_next("30 3 * * *", "Asia/Jerusalem", t)
                .ok()
                .flatten()
        };
        let mut next = next_after(clock.now_unix());
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let now = clock.now_unix();
            if next.is_some_and(|n| now >= n) {
                let failed = match retention.run(now).await {
                    Ok(report) if report.ok() => None,
                    Ok(_) => Some("DB failed the integrity check after retention cleanup"),
                    Err(_) => Some("Retention cleanup failed"),
                };
                if let Some(text) = failed {
                    events
                        .emit(agentd_core::NewEvent::new(
                            "msg",
                            "sys",
                            agentd_core::EventLevel::Error,
                            text,
                        ))
                        .await;
                }
                next = next_after(now);
            }
        }
    });
}
