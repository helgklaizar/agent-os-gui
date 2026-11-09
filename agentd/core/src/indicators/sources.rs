//! Core indicator sources: `agentd` status, unread, AI usage. The observer source is `observer_source`.

use std::sync::Arc;

use serde_json::json;

use super::{IndicatorSource, IndicatorState, Reading};
use crate::{
    EventFilter, EventStore, Observer, ObserverStatus, PortFuture, QuotaStore, StatusSource,
};

/// `agentd`: ok while the DB responds; otherwise red.
pub struct AgentdStatus(pub Arc<dyn StatusSource>);

impl IndicatorSource for AgentdStatus {
    fn id(&self) -> &str {
        "agentd"
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            match self.0.user_version().await {
                Some(v) => Reading {
                    state: IndicatorState::Ok,
                    value: Some(json!({"user_version": v})),
                    text: Some("agentd running".into()),
                },
                None => Reading {
                    state: IndicatorState::Bad,
                    value: None,
                    text: Some("DB unavailable".into()),
                },
            }
        })
    }
}

/// Unread events across the whole feed: the count in `value`, state always `ok` (no thresholds).
pub struct Unread(pub Arc<dyn EventStore>);

impl IndicatorSource for Unread {
    fn id(&self) -> &str {
        "unread"
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            match self
                .0
                .list(EventFilter {
                    limit: 1,
                    ..EventFilter::default()
                })
                .await
            {
                Ok(v) => {
                    let n = v["counts"]["total"].as_i64().unwrap_or(0);
                    Reading {
                        state: IndicatorState::Ok,
                        value: Some(json!(n)),
                        text: Some(format!("Unread: {n}")),
                    }
                }
                Err(_) => super::engine::nodata("feed unavailable"),
            }
        })
    }
}

/// AI usage: the most consumed share of a limit across accounts; >= 80 % — yellow, >= 95 % — red, no limits — `nodata`.
pub struct AiUsage(pub Arc<dyn QuotaStore>);

#[must_use]
pub(crate) fn usage_state(used: Option<f64>) -> IndicatorState {
    match used {
        None => IndicatorState::Nodata,
        Some(u) if u >= 0.95 => IndicatorState::Bad,
        Some(u) if u >= 0.8 => IndicatorState::Warn,
        Some(_) => IndicatorState::Ok,
    }
}

impl IndicatorSource for AiUsage {
    fn id(&self) -> &str {
        "ai_usage"
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            let Some(doc) = self.0.quotas(None).await else {
                return super::engine::nodata("usage unavailable");
            };
            let worst = doc["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|q| q["used"].as_f64())
                .fold(None, |m: Option<f64>, u| Some(m.map_or(u, |m| m.max(u))));
            Reading {
                state: usage_state(worst),
                value: worst.map(|u| json!(u)),
                text: None,
            }
        })
    }
}

/// Observer: available — `ok`, unavailable — red (dangerous actions are closed). Off by default.
pub struct ObserverHealth(pub Arc<dyn Observer>);

impl IndicatorSource for ObserverHealth {
    fn id(&self) -> &str {
        "observer"
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            match self.0.status() {
                ObserverStatus::Available => Reading::new(IndicatorState::Ok),
                ObserverStatus::Unavailable => Reading {
                    state: IndicatorState::Bad,
                    value: None,
                    text: Some("observer unavailable".into()),
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UsageRecord;
    use serde_json::Value;

    struct FakeQuotas(Option<Value>);

    impl QuotaStore for FakeQuotas {
        fn record(&self, _: UsageRecord) -> PortFuture<'_, Option<Value>> {
            Box::pin(async { None })
        }
        fn quotas(&self, _: Option<i64>) -> PortFuture<'_, Option<Value>> {
            let v = self.0.clone();
            Box::pin(async move { v })
        }
    }

    async fn state_of(doc: Option<Value>) -> IndicatorState {
        AiUsage(Arc::new(FakeQuotas(doc))).read().await.state
    }

    #[tokio::test]
    async fn ai_usage_takes_worst_account() {
        let doc = json!({"items": [{"used": 0.2}, {"used": 0.96}, {"used": null}]});
        assert_eq!(state_of(Some(doc)).await, IndicatorState::Bad);
        let doc = json!({"items": [{"used": 0.5}, {"used": null}]});
        assert_eq!(state_of(Some(doc)).await, IndicatorState::Ok);
        assert_eq!(
            state_of(Some(json!({"items": []}))).await,
            IndicatorState::Nodata
        );
        assert_eq!(state_of(None).await, IndicatorState::Nodata);
    }

    #[test]
    fn usage_thresholds() {
        assert_eq!(usage_state(None), IndicatorState::Nodata);
        assert_eq!(usage_state(Some(0.79)), IndicatorState::Ok);
        assert_eq!(usage_state(Some(0.8)), IndicatorState::Warn);
        assert_eq!(usage_state(Some(0.95)), IndicatorState::Bad);
    }
}
