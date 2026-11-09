//! Healthchecks heartbeat (A7-10, D43): `agentd serve` sends a `GET` to `AGENTD_HEARTBEAT_URL` every `AGENTD_HEARTBEAT_SECS` (default 300).
//! If the address is unset or not `http(s)`, the heartbeat is disabled and sends nothing. The address contains
//! the check identifier, so it never reaches logs or diagnostics: only "enabled" or "disabled".

use std::time::Duration;

const DEFAULT_SECS: u64 = 300;

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    url: String,
    every: Duration,
}

pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<Config> {
    let url = env("AGENTD_HEARTBEAT_URL")
        .map(|u| u.trim().to_string())
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))?;
    let secs = env("AGENTD_HEARTBEAT_SECS")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_SECS);
    Some(Config {
        url,
        every: Duration::from_secs(secs),
    })
}

/// `agentd diagnose` report line.
#[must_use]
pub fn report_line(cfg: Option<&Config>) -> String {
    match cfg {
        Some(_) => "heartbeat: enabled".to_string(),
        None => "heartbeat: disabled".to_string(),
    }
}

/// Starts the background ping; the first one is immediate.
pub fn spawn(cfg: Config, http: agentd_providers::HttpClient) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match http.get_status(&cfg.url, &[]).await {
                Ok(s) if (200..300).contains(&s) => {}
                Ok(s) => {
                    tracing::warn!(
                        scope = "heartbeat",
                        event = "ping_rejected",
                        status = s,
                        "heartbeat rejected"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        scope = "heartbeat",
                        event = "ping_failed",
                        "heartbeat not sent"
                    );
                }
            }
            tokio::time::sleep(cfg.every).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(
        url: Option<&'static str>,
        secs: Option<&'static str>,
    ) -> impl Fn(&str) -> Option<String> {
        move |k| match k {
            "AGENTD_HEARTBEAT_URL" => url.map(str::to_string),
            "AGENTD_HEARTBEAT_SECS" => secs.map(str::to_string),
            _ => None,
        }
    }

    #[test]
    fn disabled_without_valid_url_and_default_period() {
        assert_eq!(from_env(&env(None, None)), None);
        assert_eq!(from_env(&env(Some("  "), None)), None);
        assert_eq!(from_env(&env(Some("ftp://x/y"), None)), None);
        let c = from_env(&env(Some("https://hc.test/ping/x"), None)).unwrap();
        assert_eq!(c.every, Duration::from_secs(300));
        let c = from_env(&env(Some("http://h/p"), Some("7"))).unwrap();
        assert_eq!(c.every, Duration::from_secs(7));
        let c = from_env(&env(Some("http://h/p"), Some("0"))).unwrap();
        assert_eq!(c.every, Duration::from_secs(300), "zero is not a period");
    }
}
