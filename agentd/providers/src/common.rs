//! Shared by providers: key from the store, URL building, event sending.

use agentd_core::{ModelAccount, ProviderError, ProviderEvent, Secret, SecretStore};
use tokio::sync::mpsc;

/// The account key is fetched from the store at request time and lives only on the call stack.
pub(crate) async fn key_for(
    secrets: &dyn SecretStore,
    account: &ModelAccount,
    required: bool,
) -> Result<Option<Secret<String>>, ProviderError> {
    let key = match &account.secret_ref {
        Some(r) => secrets.get(r).await,
        None => None,
    };
    if key.is_none() && required {
        return Err(ProviderError::new("no_key", false));
    }
    Ok(key)
}

pub(crate) fn join_url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

/// `false` if the receiver is gone (the reply was stopped): no point reading on.
pub(crate) async fn emit(tx: &mpsc::Sender<ProviderEvent>, ev: ProviderEvent) -> bool {
    tx.send(ev).await.is_ok()
}
