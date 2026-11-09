//! OS notification interception (A7-05): a monitor on the user's session bus (`BecomeMonitor`) sees calls to
//! `org.freedesktop.Notifications.Notify`, each is turned into a `msg.sys` event (`source = agent-sys`) through a rate
//! limiter. No bus (no session, monitor mode refused) means interception is disabled with a journal entry, the core keeps running;
//! retry once every 30 seconds. Notification texts are untrusted: they are not parsed and not logged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agentd_core::{Clock, Events, Notification, NotifyLimiter};
use futures_util::StreamExt;
use zbus::{Connection, MessageStream, OwnedVariant};

const RULE: &str = "type='method_call',interface='org.freedesktop.Notifications',member='Notify'";
const RETRY: Duration = Duration::from_secs(30);

type Notify = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, OwnedVariant>,
    i32,
);

/// Limit window: one minute; in a test build it is set by `AGENTD_NOTIFY_WINDOW_SECS`.
fn window_secs() -> i64 {
    agentd_core::manual_clock(0)
        .and(std::env::var("AGENTD_NOTIFY_WINDOW_SECS").ok())
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(agentd_core::NOTIFY_WINDOW_SECS)
}

fn parse(body: Notify) -> Notification {
    let (app, _, _, summary, text, _, hints, _) = body;
    let urgency = hints
        .get("urgency")
        .and_then(|v| u8::try_from(v).ok())
        .unwrap_or(1);
    Notification {
        app,
        summary,
        body: text,
        urgency,
    }
}

async fn watch(
    events: &Events,
    clock: &Arc<dyn Clock>,
    limiter: &mut NotifyLimiter,
) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus.Monitoring"),
        "BecomeMonitor",
        &(&[RULE][..], 0u32),
    )
    .await?;
    tracing::info!(
        scope = "dbus",
        event = "monitor_started",
        "notification interception enabled"
    );
    let mut stream = MessageStream::from(&conn);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(msg) = msg else { return Ok(()) };
                let msg = msg?;
                let header = msg.header();
                                tracing::debug!(scope = "dbus", event = "message_seen", member = ?header.member().map(|m| m.as_str().to_string()), interface = ?header.interface().map(|m| m.as_str().to_string()), "message on the bus");
                let is_notify = header.member().is_some_and(|m| m.as_str() == "Notify")
                    && header.interface().is_some_and(|i| i.as_str() == "org.freedesktop.Notifications");
                if !is_notify {
                    continue;
                }
                let body = match msg.body().deserialize::<Notify>() {
                    Ok(b) => b,
                    Err(e) => {
                                                tracing::debug!(scope = "dbus", event = "notify_unparsed", error = %e, "notification not parsed");
                        continue;
                    }
                };
                for e in limiter.admit(&parse(body), clock.now_unix()) {
                    drop(events.emit(e).await);
                }
            }
            _ = tick.tick() => {
                for e in limiter.flush(clock.now_unix()) {
                    drop(events.emit(e).await);
                }
            }
        }
    }
}

/// Starts the background interception task.
pub(crate) fn spawn(events: Events, clock: Arc<dyn Clock>) {
    tokio::spawn(async move {
        let mut limiter = NotifyLimiter::new(window_secs());
        let mut warned = false;
        loop {
            if let Err(e) = watch(&events, &clock, &mut limiter).await
                && !warned
            {
                tracing::warn!(
                    scope = "dbus",
                    event = "monitor_unavailable",
                    error = %e,
                                        "session bus unavailable, notification interception disabled"
                );
                warned = true;
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}
