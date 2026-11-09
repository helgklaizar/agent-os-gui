//! Product provider registry.

mod common;

use std::sync::Arc;

use agentd_core::{ProviderKind, SystemClock};
use agentd_providers::{HttpClient, Timeouts, standard_registry};

fn reg() -> agentd_core::ProviderRegistry {
    let k = Arc::new(agentd_providers::KeysFile::new("/nonexistent/keys.env"));
    standard_registry(
        k,
        Arc::new(SystemClock),
        HttpClient::new(Timeouts::default()),
    )
}

#[test]
fn registry_has_real_providers() {
    let r = reg();
    for k in [
        ProviderKind::Anthropic,
        ProviderKind::OpenAiCompat,
        ProviderKind::Local,
    ] {
        assert!(r.get(k).is_some(), "{k:?}");
    }
}

#[cfg(not(feature = "test-support"))]
#[test]
fn scripted_provider_absent_in_release() {
    assert!(reg().get(ProviderKind::Scripted).is_none());
}

#[cfg(feature = "test-support")]
#[test]
fn scripted_provider_present_in_test_build() {
    assert!(reg().get(ProviderKind::Scripted).is_some());
}
