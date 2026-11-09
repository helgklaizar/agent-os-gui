//! Registration of WS topics for the connected services (docs/reference/api.md 4.3).

use std::sync::Arc;

use agentd_core::Scope;

use crate::hub::{Hub, SettingsTopic};
use crate::state::Services;

pub(crate) fn register(hub: &Hub, services: &Services) {
    if let Some(settings) = services.settings.clone() {
        hub.register_topic("settings", Scope::Read, Arc::new(SettingsTopic(settings)));
    }
    if let Some(tree) = services.tree.clone() {
        crate::tree::register_topics(hub, tree);
    }
    if let Some(chat) = services.chat.clone() {
        hub.register_topic(
            "session.",
            Scope::Read,
            Arc::new(crate::chat::SessionTopic(chat)),
        );
    }
    if let Some(events) = services.events.clone() {
        hub.register_topic(
            "events",
            Scope::Read,
            Arc::new(crate::events::EventsTopic(events)),
        );
    }
    if let Some(engine) = services.indicators.clone() {
        hub.register_topic(
            "bar.indicators",
            Scope::Read,
            Arc::new(crate::events::IndicatorsTopic(engine)),
        );
    }
    if let Some(m) = services.modules.clone() {
        hub.register_topic(
            "modules",
            Scope::Read,
            Arc::new(crate::modules::ModulesTopic(m)),
        );
    }
    if let Some(sys) = services.sys.clone() {
        hub.register_topic("bar.sys", Scope::Read, Arc::new(SysTopic(sys)));
    }
    if let Some(w) = services.weather.clone() {
        hub.register_topic("bar.weather", Scope::Read, Arc::new(WeatherTopic(w)));
    }
    if let Some(q) = services.quotas.clone() {
        hub.register_topic(
            "bar.quota",
            Scope::Read,
            Arc::new(crate::quotas::QuotaTopic(q)),
        );
    }
}

/// `snapshot` of the `bar.sys` topic: the latest sample.
struct SysTopic(agentd_core::SysSnapshot);

impl crate::hub::TopicSource for SysTopic {
    fn snapshot<'a>(
        &'a self,
        _topic: &'a str,
    ) -> agentd_core::PortFuture<'a, Option<serde_json::Value>> {
        Box::pin(async move { Some(self.0.get()) })
    }
}

/// `snapshot` of the `bar.weather` topic: the latest frame.
struct WeatherTopic(agentd_core::WeatherSnapshot);

impl crate::hub::TopicSource for WeatherTopic {
    fn snapshot<'a>(
        &'a self,
        _topic: &'a str,
    ) -> agentd_core::PortFuture<'a, Option<serde_json::Value>> {
        Box::pin(async move { Some(self.0.get()) })
    }
}
