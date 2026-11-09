//! JSON exports of the configuration (A6-11, docs/reference/db.md section 1): `config.json`, `connections.json`, `personas.json`, `modules.json`,
//! `tunnels.json` in `~/.agentos`. The core regenerates them when data changes (atomically, `0600`) and never reads
//! them back: editing a file by hand does not affect state. There are no secrets in the data (values live only in `keys.env`).

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agentd_core::{ModuleStore, SettingsStore, TreeStore, TunnelStore};
use serde_json::{Value, json};

pub(crate) struct Sources {
    pub settings: Arc<dyn SettingsStore>,
    pub tree: Arc<dyn TreeStore>,
    pub modules: Arc<dyn ModuleStore>,
    pub tunnels: Arc<dyn TunnelStore>,
}

async fn page(s: &dyn SettingsStore, name: &str) -> Value {
    s.get(name).await.map_or(Value::Null, |d| d.doc)
}

async fn render(src: &Sources) -> Vec<(&'static str, Value)> {
    let s = src.settings.as_ref();
    let config = json!({
        "account": page(s, "account").await, "models": page(s, "models").await,
        "schedule": page(s, "schedule").await, "security": page(s, "security").await,
        "memory": page(s, "memory").await, "tunnels": page(s, "tunnels").await,
        "permissions": page(s, "permissions").await,
    });
    let connections = json!({
        "page": page(s, "connections").await,
        "accounts": src.tree.list_accounts().await.unwrap_or(Value::Null),
    });
    let personas = src.tree.list_personas().await.unwrap_or(Value::Null);
    let modules: Vec<Value> = src
        .modules
        .list()
        .await
        .unwrap_or_default()
        .iter()
        .map(agentd_core::Module::to_json)
        .collect();
    let tunnels: Vec<Value> = src
        .tunnels
        .list()
        .await
        .unwrap_or_default()
        .iter()
        .map(agentd_core::Tunnel::to_json)
        .collect();
    vec![
        ("config.json", config),
        ("connections.json", connections),
        ("personas.json", personas),
        ("modules.json", Value::Array(modules)),
        ("tunnels.json", Value::Array(tunnels)),
    ]
}

fn write_private(path: &PathBuf, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Background reconciliation every `every`: a file is written only when the content changed according to the DB data.
pub(crate) fn spawn(root: PathBuf, src: Sources, every: Duration) {
    tokio::spawn(async move {
        let mut last: HashMap<&'static str, String> = HashMap::new();
        loop {
            for (name, doc) in render(&src).await {
                let text = serde_json::to_string_pretty(&doc).unwrap_or_default();
                if last.get(name) != Some(&text) {
                    let path = root.join(name);
                    match write_private(&path, &text) {
                        Ok(()) => {
                            last.insert(name, text);
                        }
                        Err(e) => {
                            tracing::warn!(scope = "export", event = "write_failed", name, kind = ?e.kind(), "export not written");
                        }
                    }
                }
            }
            tokio::time::sleep(every).await;
        }
    });
}
