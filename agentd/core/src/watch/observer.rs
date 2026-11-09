//! `ModelObserver`: rules + model. Availability follows the last model call (heartbeat or request).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{Action, Hit, Observer, ObserverStatus, PortFuture};

use super::classifier::{Classifier, Subject};

pub struct ModelObserver {
    rules: Arc<dyn Observer>,
    classifier: Arc<dyn Classifier>,
    /// The last model call succeeded. Before the first heartbeat — `false`: until checked, dangerous actions are closed.
    healthy: AtomicBool,
    /// Test build only: forced unavailability.
    #[cfg(feature = "test-support")]
    forced_down: AtomicBool,
}

impl ModelObserver {
    #[must_use]
    pub fn new(rules: Arc<dyn Observer>, classifier: Arc<dyn Classifier>) -> ModelObserver {
        ModelObserver {
            rules,
            classifier,
            healthy: AtomicBool::new(false),
            #[cfg(feature = "test-support")]
            forced_down: AtomicBool::new(false),
        }
    }

    /// Availability heartbeat: a harmless model request every `every`; the result updates the state.
    pub fn spawn_heartbeat(self: &Arc<Self>, every: Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let ok = me
                    .classifier
                    .classify(Subject::Command, "echo ok")
                    .await
                    .is_ok();
                if me.healthy.swap(ok, Ordering::SeqCst) != ok {
                    tracing::warn!(
                        scope = "observer",
                        event = if ok { "watch_up" } else { "watch_down" },
                        "model-based observer availability changed"
                    );
                }
                tokio::time::sleep(every).await;
            }
        });
    }
}

fn subject_of(action: Action) -> Option<Subject> {
    match action {
        Action::Shell => Some(Subject::Command),
        Action::Browse => Some(Subject::Url),
        Action::Send => Some(Subject::Message),
        _ => None,
    }
}

impl Observer for ModelObserver {
    fn status(&self) -> ObserverStatus {
        #[cfg(feature = "test-support")]
        if self.forced_down.load(Ordering::SeqCst) {
            return ObserverStatus::Unavailable;
        }
        if self.rules.status() == ObserverStatus::Available && self.healthy.load(Ordering::SeqCst) {
            ObserverStatus::Available
        } else {
            ObserverStatus::Unavailable
        }
    }

    fn check_action(&self, action: Action, target: &str) -> Option<Hit> {
        self.rules.check_action(action, target)
    }

    fn check_text(&self, text: &str) -> Option<Hit> {
        self.rules.check_text(text)
    }

    fn classify<'a>(&'a self, action: Action, target: &'a str) -> PortFuture<'a, Option<Hit>> {
        Box::pin(async move {
            let subject = subject_of(action)?;
            match self.classifier.classify(subject, target).await {
                Ok(v) => {
                    self.healthy.store(true, Ordering::SeqCst);
                    v.hit()
                }
                Err(e) => {
                    // The model did not answer: the observer is unavailable, the action is closed with a request (fail-closed).
                    self.healthy.store(false, Ordering::SeqCst);
                    tracing::warn!(
                        scope = "observer",
                        event = "watch_classify_failed",
                        code = e.0,
                        "observer model did not answer"
                    );
                    Some(Hit {
                        kind: "dangerous_cmd",
                        severity: "med",
                        verdict: "asked",
                        rule: format!("watch_unavailable:{}", e.0),
                    })
                }
            }
        })
    }

    #[cfg(feature = "test-support")]
    fn force_available(&self, available: bool) -> bool {
        self.forced_down.store(!available, Ordering::SeqCst);
        true
    }
}
