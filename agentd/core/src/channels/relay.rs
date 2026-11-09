//! Provider registry and send relay: the first provider with the needed capability and health not `Down`; moving to the next
//! only on a confirmed `Rejected`; `Unknown` stops the send without a retry.

use std::sync::Arc;

use super::{Capability, ChannelAccount, ChannelProvider, Health, Outcome};

#[derive(Clone, Default)]
pub struct ChannelRegistry {
    providers: Vec<Arc<dyn ChannelProvider>>,
}

impl ChannelRegistry {
    #[must_use]
    pub fn with(mut self, p: Arc<dyn ChannelProvider>) -> ChannelRegistry {
        self.providers.push(p);
        self
    }

    /// Providers of the account type in registration order.
    #[must_use]
    pub fn for_kind(&self, kind: &str) -> Vec<Arc<dyn ChannelProvider>> {
        self.providers
            .iter()
            .filter(|p| p.kind() == kind)
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn first(&self, kind: &str) -> Option<Arc<dyn ChannelProvider>> {
        self.providers.iter().find(|p| p.kind() == kind).cloned()
    }

    /// Send relay; `None` — no suitable provider (all `Down` or without the `Send` capability).
    pub async fn send(
        &self,
        account: &ChannelAccount,
        to: &str,
        text: &str,
        key: &str,
    ) -> Option<Outcome> {
        let mut last = None;
        for p in self.for_kind(&account.kind) {
            if !p.capabilities().contains(&Capability::Send)
                || p.health(account).await == Health::Down
            {
                continue;
            }
            match p.send(account, to, text, key).await {
                Outcome::Rejected(code) => last = Some(Outcome::Rejected(code)),
                other => return Some(other),
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::PortFuture;
    use crate::channels::{ConnectKind, Flow, FlowStatus, Incoming};

    struct Fake {
        name: &'static str,
        health: Health,
        out: Outcome,
        sent: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ChannelProvider for Fake {
        fn kind(&self) -> &'static str {
            "tg"
        }
        fn capabilities(&self) -> &'static [Capability] {
            &[Capability::Send]
        }
        fn connect_kind(&self) -> ConnectKind {
            ConnectKind::Token
        }
        fn connect<'a>(&'a self, _: &'a ChannelAccount) -> PortFuture<'a, Flow> {
            Box::pin(async { Flow::new(ConnectKind::Token, FlowStatus::Connected) })
        }
        fn health<'a>(&'a self, _: &'a ChannelAccount) -> PortFuture<'a, Health> {
            let h = self.health;
            Box::pin(async move { h })
        }
        fn send<'a>(
            &'a self,
            _: &'a ChannelAccount,
            _: &'a str,
            _: &'a str,
            _: &'a str,
        ) -> PortFuture<'a, Outcome> {
            self.sent.lock().unwrap().push(self.name);
            let o = self.out.clone();
            Box::pin(async move { o })
        }
        fn poll<'a>(&'a self, _: &'a ChannelAccount) -> PortFuture<'a, Vec<Incoming>> {
            Box::pin(async { Vec::new() })
        }
    }

    fn acc() -> ChannelAccount {
        ChannelAccount {
            id: 1,
            kind: "tg".into(),
        }
    }

    fn reg(
        list: &[(&'static str, Health, Outcome)],
    ) -> (ChannelRegistry, Arc<Mutex<Vec<&'static str>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut r = ChannelRegistry::default();
        for (name, health, out) in list {
            r = r.with(Arc::new(Fake {
                name,
                health: *health,
                out: out.clone(),
                sent: sent.clone(),
            }));
        }
        (r, sent)
    }

    #[tokio::test]
    async fn relay_switches_only_on_confirmed_rejection() {
        let (r, sent) = reg(&[
            ("a", Health::Ok, Outcome::Rejected("blocked")),
            ("b", Health::Ok, Outcome::Delivered { ext_id: None }),
        ]);
        assert_eq!(
            r.send(&acc(), "@x", "t", "k").await,
            Some(Outcome::Delivered { ext_id: None })
        );
        assert_eq!(*sent.lock().unwrap(), ["a", "b"]);
    }

    #[tokio::test]
    async fn unknown_outcome_stops_relay_without_retry() {
        let (r, sent) = reg(&[
            ("a", Health::Ok, Outcome::Unknown),
            ("b", Health::Ok, Outcome::Delivered { ext_id: None }),
        ]);
        assert_eq!(r.send(&acc(), "@x", "t", "k").await, Some(Outcome::Unknown));
        assert_eq!(
            *sent.lock().unwrap(),
            ["a"],
            "the second provider was not called"
        );
    }

    #[tokio::test]
    async fn down_provider_is_skipped_and_all_rejected_reports_rejection() {
        let (r, sent) = reg(&[
            ("a", Health::Down, Outcome::Delivered { ext_id: None }),
            ("b", Health::Degraded, Outcome::Rejected("bad_chat")),
        ]);
        assert_eq!(
            r.send(&acc(), "@x", "t", "k").await,
            Some(Outcome::Rejected("bad_chat"))
        );
        assert_eq!(*sent.lock().unwrap(), ["b"]);
        let (r, _) = reg(&[("a", Health::Down, Outcome::Unknown)]);
        assert_eq!(r.send(&acc(), "@x", "t", "k").await, None);
    }
}
