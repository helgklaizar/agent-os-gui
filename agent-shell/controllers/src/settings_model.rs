//! Unified Settings data model: everything the pages show lives here once and comes from the core.
//! Views keep no copies: "Connections", "Models" and "Personas" read the same lists.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::bar_types::{QuotaItem, QuotaKind, SysFrame};
use crate::permissions::PermTable;
use crate::settings_types::{FileEntry, Reply};

/// Channel account types (non-AI): messengers and mail.
const CHANNEL_TYPES: [&str; 6] = ["wa", "tg", "gmail", "mail", "imap", "whapi"];
/// Unofficial channel types: the channel itself lives in a module outside 0.1, and the UI marks it.
const UNOFFICIAL: [&str; 2] = ["wa", "whapi"];

/// A core page: the document without `rev`, and `rev`.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub value: Value,
    pub rev: i64,
}

#[derive(Debug, Clone, Default)]
pub struct CoreModel {
    pub docs: BTreeMap<String, Doc>,
    pub personas: Vec<Value>,
    pub accounts: Vec<Value>,
    pub modules: Vec<Value>,
    pub tunnels: Vec<Value>,
    pub files: Vec<FileEntry>,
    pub secrets: BTreeSet<String>,
    pub perms: PermTable,
    pub quotas: Vec<QuotaItem>,
    pub sys: SysFrame,
    /// At least one core response has been received.
    pub loaded: bool,
}

pub fn id_of(v: &Value) -> Option<i64> {
    v.get("id").and_then(Value::as_i64)
}

pub fn text_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn is_channel(account: &Value) -> bool {
    CHANNEL_TYPES.contains(&text_of(account, "type"))
}

pub fn is_unofficial(account: &Value) -> bool {
    UNOFFICIAL.contains(&text_of(account, "type"))
}

impl CoreModel {
    /// Apply a core response. `true` means the model changed.
    pub fn apply(&mut self, reply: &Reply) -> bool {
        self.loaded = true;
        match reply {
            Reply::Page {
                doc, value, rev, ..
            } => {
                self.docs.insert(
                    doc.clone(),
                    Doc {
                        value: value.clone(),
                        rev: *rev,
                    },
                );
            }
            Reply::Personas(v) => self.personas = v.clone(),
            Reply::Accounts(v) => self.accounts = v.clone(),
            Reply::Modules(v) => self.modules = v.clone(),
            Reply::Tunnels(v) => self.tunnels = v.clone(),
            Reply::Files(v) => self.files = v.clone(),
            Reply::Secrets(v) => self.secrets = v.iter().cloned().collect(),
            Reply::Perms(v) => self.perms = PermTable::from_value(v),
            _ => return false,
        }
        true
    }

    pub fn doc(&self, name: &str) -> Option<&Doc> {
        self.docs.get(name)
    }

    pub fn rev(&self, name: &str) -> i64 {
        self.docs.get(name).map_or(0, |d| d.rev)
    }

    /// Value by dotted path (`balance.fallback`, `accounts.3.model`).
    pub fn at(&self, doc: &str, path: &str) -> Option<&Value> {
        let mut cur = &self.doc(doc)?.value;
        for part in path.split('.') {
            cur = cur.get(part)?;
        }
        (!cur.is_null()).then_some(cur)
    }

    /// AI provider accounts.
    pub fn ai_accounts(&self) -> Vec<&Value> {
        self.accounts.iter().filter(|a| !is_channel(a)).collect()
    }

    pub fn persona(&self, id: i64) -> Option<&Value> {
        self.personas.iter().find(|p| id_of(p) == Some(id))
    }

    pub fn account_label(&self, id: i64) -> String {
        self.accounts
            .iter()
            .find(|a| id_of(a) == Some(id))
            .map(|a| text_of(a, "label").to_string())
            .unwrap_or_default()
    }

    /// Local models from `bar.quota`: the same source as the bar.
    pub fn local_models(&self) -> Vec<&QuotaItem> {
        self.quotas
            .iter()
            .filter(|q| q.kind == QuotaKind::Local)
            .collect()
    }

    pub fn secret_set(&self, reference: &str) -> bool {
        self.secrets.contains(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn at_walks_dotted_path_and_skips_null() {
        let mut m = CoreModel::default();
        m.apply(&Reply::Page {
            doc: "personas".into(),
            value: json!({"balance": {"fallback": "wait", "x": null}}),
            rev: 4,
            saved: false,
        });
        assert_eq!(m.at("personas", "balance.fallback"), Some(&json!("wait")));
        assert_eq!(m.at("personas", "balance.x"), None);
        assert_eq!(m.at("personas", "balance.nope"), None);
        assert_eq!(m.at("models", "a"), None);
        assert_eq!(m.rev("personas"), 4);
        assert_eq!(m.rev("models"), 0);
    }

    #[test]
    fn channels_and_unofficial_are_told_apart() {
        assert!(is_channel(&json!({"type": "tg"})));
        assert!(!is_channel(&json!({"type": "anthropic"})));
        assert!(is_unofficial(&json!({"type": "wa"})));
        assert!(!is_unofficial(&json!({"type": "tg"})));
    }

    #[test]
    fn apply_ignores_replies_without_data() {
        let mut m = CoreModel::default();
        assert!(!m.apply(&Reply::ModulePending));
        assert!(m.apply(&Reply::Secrets(vec!["account.3".into()])));
        assert!(m.secret_set("account.3"));
        assert!(!m.secret_set("account.4"));
    }
}
