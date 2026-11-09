//! Product provider registry.

use std::sync::Arc;

use agentd_core::{Clock, ProviderKind, ProviderRegistry, SecretStore};

use crate::HttpClient;
use crate::anthropic::Anthropic;
use crate::openai::OpenAiCompat;

/// Anthropic, OpenAI-compatible and local. The `scripted` provider exists only in the `test-support` build.
#[must_use]
pub fn standard_registry(
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    http: HttpClient,
) -> ProviderRegistry {
    let reg = ProviderRegistry::new()
        .with(
            ProviderKind::Anthropic,
            Arc::new(Anthropic::new(http.clone(), secrets.clone(), clock.clone())),
        )
        .with(
            ProviderKind::OpenAiCompat,
            Arc::new(OpenAiCompat::new(
                http.clone(),
                secrets.clone(),
                clock.clone(),
                false,
            )),
        )
        .with(
            ProviderKind::Local,
            Arc::new(OpenAiCompat::new(http, secrets, clock, true)),
        );
    #[cfg(feature = "test-support")]
    let reg = reg.with(
        ProviderKind::Scripted,
        Arc::new(crate::scripted::Scripted::new()),
    );
    reg
}
