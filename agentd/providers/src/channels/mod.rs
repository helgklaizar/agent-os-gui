//! Channel providers: Telegram Bot API (the reference channel) and the test `scripted-channel` (`test-support` only).

mod telegram;
mod whapi;
pub use telegram::{DEFAULT_BASE_URL, Telegram};
pub use whapi::{WHAPI_BASE_URL, Whapi};

#[cfg(feature = "test-support")]
mod scripted;
#[cfg(feature = "test-support")]
pub use scripted::ScriptedChannel;

use std::sync::Arc;

use agentd_core::{ChannelRegistry, SecretStore};

use crate::HttpClient;

/// Product providers; `scripted-channel` exists only in the `test-support` build.
#[must_use]
pub fn standard_channels(secrets: Arc<dyn SecretStore>, http: HttpClient) -> ChannelRegistry {
    ChannelRegistry::default()
        .with(Arc::new(Telegram::new(
            http.clone(),
            secrets.clone(),
            DEFAULT_BASE_URL,
        )))
        .with(Arc::new(Whapi::new(http, secrets, WHAPI_BASE_URL)))
}

/// Test channel and its control; `None` without `test-support`.
#[must_use]
pub fn scripted_channel(
    secrets: Arc<dyn SecretStore>,
) -> Option<(
    Arc<dyn agentd_core::ChannelProvider>,
    Arc<dyn agentd_core::ChannelControl>,
)> {
    #[cfg(feature = "test-support")]
    {
        let c = Arc::new(ScriptedChannel::new(secrets));
        Some((c.clone(), c))
    }
    #[cfg(not(feature = "test-support"))]
    {
        drop(secrets);
        None
    }
}
