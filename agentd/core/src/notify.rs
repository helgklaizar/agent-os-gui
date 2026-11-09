//! OS notifications: converting a D-Bus notification into a `msg.sys` event (`source = agent-sys`) and rate
//! limiting: at most `PER_WINDOW` events per application per window (a minute), the rest is collapsed into one event
//! "N notifications". Texts are untrusted: not edited and not parsed, the event field limits the length.

use std::collections::HashMap;

use crate::{EventLevel, NewEvent};

/// Events per application per window.
pub const PER_WINDOW: u32 = 20;
/// Default window, seconds.
pub const WINDOW_SECS: i64 = 60;
/// Notification event source.
pub const SOURCE: &str = "agent-sys";

/// `Notify` notification from `org.freedesktop.Notifications`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub app: String,
    pub summary: String,
    pub body: String,
    /// `urgency` from hints: 0 low, 1 normal, 2 critical; no hint — 1.
    pub urgency: u8,
}

fn level_of(urgency: u8) -> EventLevel {
    if urgency >= 2 {
        EventLevel::Warn
    } else {
        EventLevel::Info
    }
}

fn app_label(app: &str) -> &str {
    if app.trim().is_empty() { "system" } else { app }
}

impl Notification {
    #[must_use]
    pub fn to_event(&self) -> NewEvent {
        let mut text = format!("{}: {}", app_label(&self.app), self.summary);
        if !self.body.is_empty() {
            text.push('\n');
            text.push_str(&self.body);
        }
        let mut e = NewEvent::new("msg", "sys", level_of(self.urgency), text);
        e.source = Some(SOURCE.to_string());
        e
    }
}

struct Window {
    start: i64,
    emitted: u32,
    folded: u32,
    urgency: u8,
}

/// Summary of a closed window: one event "N notifications" if anything was collapsed.
fn summary(app: &str, w: &Window) -> Option<NewEvent> {
    if w.folded == 0 {
        return None;
    }
    let text = format!(
        "{}: {} notifications (collapsed, limit {PER_WINDOW} per window)",
        app_label(app),
        w.folded
    );
    let mut e = NewEvent::new("msg", "sys", level_of(w.urgency), text);
    e.source = Some(SOURCE.to_string());
    Some(e)
}

/// Per-application rate limiter.
pub struct Limiter {
    window_secs: i64,
    apps: HashMap<String, Window>,
}

impl Limiter {
    #[must_use]
    pub fn new(window_secs: i64) -> Limiter {
        Limiter {
            window_secs: window_secs.max(1),
            apps: HashMap::new(),
        }
    }

    /// A notification at moment `now`: the events to record (empty — collapsed into the window counter). If the notification
    /// opens a new window, the summary of the past one comes first.
    pub fn admit(&mut self, n: &Notification, now: i64) -> Vec<NewEvent> {
        let mut out = Vec::new();
        let w = self.apps.entry(n.app.clone()).or_insert(Window {
            start: now,
            emitted: 0,
            folded: 0,
            urgency: 1,
        });
        if now - w.start >= self.window_secs {
            out.extend(summary(&n.app, w));
            *w = Window {
                start: now,
                emitted: 0,
                folded: 0,
                urgency: 1,
            };
        }
        if w.emitted < PER_WINDOW {
            w.emitted += 1;
            out.push(n.to_event());
        } else {
            w.folded += 1;
            w.urgency = w.urgency.max(n.urgency);
        }
        out
    }

    /// Summary events for windows that ended by moment `now`; closed windows are forgotten.
    pub fn flush(&mut self, now: i64) -> Vec<NewEvent> {
        let mut out = Vec::new();
        let window = self.window_secs;
        self.apps.retain(|app, w| {
            if now - w.start < window {
                return true;
            }
            out.extend(summary(app, w));
            false
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(app: &str, urgency: u8) -> Notification {
        Notification {
            app: app.into(),
            summary: "subject".into(),
            body: String::new(),
            urgency,
        }
    }

    #[test]
    fn event_has_source_level_and_verbatim_text() {
        let evil = Notification {
            app: "x".into(),
            summary: "<b>&amp;</b>".into(),
            body: "line\n'; DROP TABLE events;--".into(),
            urgency: 2,
        };
        let e = evil.to_event();
        assert_eq!(e.source.as_deref(), Some("agent-sys"));
        assert_eq!((e.kind, e.sub.as_str()), ("msg", "sys"));
        assert_eq!(e.level, EventLevel::Warn);
        assert_eq!(e.text, "x: <b>&amp;</b>\nline\n'; DROP TABLE events;--");
        assert_eq!(n("", 1).to_event().text, "system: subject");
        assert_eq!(n("a", 0).to_event().level, EventLevel::Info);
    }

    #[test]
    fn twenty_pass_the_rest_fold_into_one_summary() {
        let mut l = Limiter::new(60);
        let passed: usize = (0..100).map(|_| l.admit(&n("spam", 1), 1000).len()).sum();
        assert_eq!(passed, 20);
        assert!(l.flush(1059).is_empty(), "the window is still open");
        let s = l.flush(1060);
        assert_eq!(s.len(), 1);
        assert!(s[0].text.contains("80 notifications"), "{}", s[0].text);
        assert!(l.flush(2000).is_empty(), "the summary is issued once");
    }

    #[test]
    fn apps_and_windows_are_independent() {
        let mut l = Limiter::new(60);
        for _ in 0..25 {
            l.admit(&n("a", 1), 0);
        }
        assert_eq!(
            l.admit(&n("b", 1), 0).len(),
            1,
            "another application is unaffected"
        );
        // First notification of a new window: the summary of the past one first, then the event itself.
        let e = l.admit(&n("a", 1), 61);
        assert_eq!(e.len(), 2);
        assert!(e[0].text.contains("5 notifications"), "{}", e[0].text);
        assert_eq!(e[1].text, "a: subject");
        assert_eq!(l.flush(61).len(), 0, "window `a` is already refreshed");
    }

    #[test]
    fn folded_urgent_keeps_summary_warning() {
        let mut l = Limiter::new(60);
        for _ in 0..20 {
            l.admit(&n("a", 1), 0);
        }
        l.admit(&n("a", 2), 1);
        let s = l.flush(60);
        assert_eq!(s[0].level, EventLevel::Warn);
    }
}
