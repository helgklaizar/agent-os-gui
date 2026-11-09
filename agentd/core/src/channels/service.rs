//! Channel account connection, incoming polling and channel health in the bar.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::{
    Capability, ChannelAccount, ChannelProvider, ChannelRegistry, Flow, FlowStatus, Health,
};
use crate::{
    Clock, Direction, Gateway, IndicatorEngine, IndicatorState, Ingest, Notifier, Reading,
    TreeStore, channel_of,
};

#[derive(Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// No such account or it is deleted.
    NotFound,
    /// The account type has no channel provider (model, unknown type).
    NotChannel,
    Unavailable,
}

struct Inner {
    registry: ChannelRegistry,
    tree: Arc<dyn TreeStore>,
    gateway: Gateway,
    indicators: Option<IndicatorEngine>,
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
}

#[derive(Clone)]
pub struct Channels {
    inner: Arc<Inner>,
}

/// Account from a `TreeStore` response: `(provider description, status)`.
fn parse_account(v: &Value) -> Option<(ChannelAccount, String)> {
    Some((
        ChannelAccount {
            id: v["id"].as_i64()?,
            kind: v["type"].as_str()?.to_string(),
        },
        v["status"].as_str().unwrap_or("disconnected").to_string(),
    ))
}

impl Channels {
    #[must_use]
    pub fn new(
        registry: ChannelRegistry,
        tree: Arc<dyn TreeStore>,
        gateway: Gateway,
        indicators: Option<IndicatorEngine>,
        notifier: Arc<dyn Notifier>,
        clock: Arc<dyn Clock>,
    ) -> Channels {
        Channels {
            inner: Arc::new(Inner {
                registry,
                tree,
                gateway,
                indicators,
                notifier,
                clock,
            }),
        }
    }

    #[must_use]
    pub fn registry(&self) -> &ChannelRegistry {
        &self.inner.registry
    }

    /// Accounts with description and status; a store error gives an empty list in the log.
    pub async fn accounts(&self) -> Vec<(ChannelAccount, String)> {
        match self.inner.tree.list_accounts().await {
            Ok(Value::Array(list)) => list.iter().filter_map(parse_account).collect(),
            _ => Vec::new(),
        }
    }

    async fn find(
        &self,
        id: i64,
    ) -> Result<(ChannelAccount, Arc<dyn ChannelProvider>), ChannelError> {
        let list = self
            .inner
            .tree
            .list_accounts()
            .await
            .map_err(|_| ChannelError::Unavailable)?;
        let (acc, _) = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_account)
            .find(|(a, _)| a.id == id)
            .ok_or(ChannelError::NotFound)?;
        let provider = self
            .inner
            .registry
            .first(&acc.kind)
            .ok_or(ChannelError::NotChannel)?;
        Ok((acc, provider))
    }

    /// Records the account status, reports flow progress in `connections` and updates the channel indicator.
    async fn apply(&self, acc: &ChannelAccount, provider: &Arc<dyn ChannelProvider>, flow: &Flow) {
        if self
            .inner
            .tree
            .set_account_status(acc.id, flow.account_status())
            .await
            .is_err()
        {
            tracing::debug!(
                scope = "channels",
                event = "account_gone",
                "account disappeared"
            );
            return;
        }
        self.inner.notifier.publish_event(
            "connections",
            json!({"op": "connect_flow", "flow": flow.to_json(acc.id)}),
        );
        let state = match flow.status {
            FlowStatus::Connected => match provider.health(acc).await {
                Health::Ok => IndicatorState::Ok,
                Health::Degraded => IndicatorState::Warn,
                Health::Down => IndicatorState::Bad,
            },
            FlowStatus::Failed | FlowStatus::Expired => IndicatorState::Bad,
            _ if flow.account_status() == "qr" => IndicatorState::Bad,
            _ => IndicatorState::Nodata,
        };
        if let Some(engine) = &self.inner.indicators {
            let mut r = Reading::new(state);
            r.text = Some(format!("channel {}: {}", acc.id, flow.account_status()));
            if engine
                .push(&format!("channel.{}", acc.id), r)
                .await
                .is_err()
            {
                tracing::debug!(
                    scope = "channels",
                    event = "indicator_skipped",
                    "channel frame not accepted"
                );
            }
        }
    }

    /// `POST /accounts/{id}/connect`: start or repeat the flow; the result is `ConnectFlow`.
    pub async fn connect(&self, id: i64) -> Result<Value, ChannelError> {
        let (acc, provider) = self.find(id).await?;
        let flow = provider.connect(&acc).await;
        self.apply(&acc, &provider, &flow).await;
        Ok(flow.to_json(id))
    }

    /// `DELETE /accounts/{id}/connect`: disconnect; repeating is harmless.
    pub async fn disconnect(&self, id: i64) -> Result<(), ChannelError> {
        let (acc, provider) = self.find(id).await?;
        provider.disconnect(&acc).await;
        let mut flow = Flow::new(provider.connect_kind(), FlowStatus::Pending);
        flow.url = None;
        if self
            .inner
            .tree
            .set_account_status(id, "disconnected")
            .await
            .is_err()
        {
            return Err(ChannelError::NotFound);
        }
        self.inner.notifier.publish_event(
            "connections",
            json!({"op": "connect_flow", "flow": flow.to_json(id)}),
        );
        if let Some(engine) = &self.inner.indicators {
            let mut r = Reading::new(IndicatorState::Nodata);
            r.text = Some(format!("channel {}: disconnected", acc.id));
            drop(engine.push(&format!("channel.{}", acc.id), r).await);
        }
        Ok(())
    }

    /// Connection recheck (after a token write, on a timer, on a test command): errors are not returned.
    pub async fn refresh(&self, id: i64) {
        match self.find(id).await {
            Ok((acc, provider)) => {
                let flow = provider.connect(&acc).await;
                self.apply(&acc, &provider, &flow).await;
            }
            Err(ChannelError::NotChannel | ChannelError::NotFound) => {}
            Err(ChannelError::Unavailable) => {
                tracing::debug!(
                    scope = "channels",
                    event = "refresh_skipped",
                    "store unavailable"
                );
            }
        }
    }

    /// Polling incoming messages of connected accounts that can receive; each message goes through the gateway.
    pub async fn poll_all(&self) -> usize {
        let mut taken = 0;
        for (acc, status) in self.accounts().await {
            let Some(p) = self.inner.registry.first(&acc.kind) else {
                continue;
            };
            if status != "connected" || !p.capabilities().contains(&Capability::Receive) {
                continue;
            }
            for m in p.poll(&acc).await {
                let input = Ingest {
                    account_id: acc.id,
                    ext_id: m.ext_id,
                    direction: Direction::In,
                    handle: m.handle,
                    body: m.body,
                    ts: self.inner.clock.now_unix(),
                };
                if self.inner.gateway.ingest(input).await.is_ok() {
                    taken += 1;
                }
            }
        }
        taken
    }

    /// Background polling with period `every`.
    pub fn spawn(&self, every: Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            loop {
                t.tick().await;
                me.poll_all().await;
            }
        });
    }

    /// Channel subtype for events and send targets.
    #[must_use]
    pub fn channel_name(account_type: &str) -> String {
        channel_of(account_type)
    }
}
