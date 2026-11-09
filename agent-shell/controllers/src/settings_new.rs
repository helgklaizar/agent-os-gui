//! Settings "new record" form fields: account, MCP server, tunnel, module address, file text.

use crate::i18n::tr;
use crate::settings_entities::entity;
use crate::settings_fields::{Spec, Ty};

/// Account types that can be added.
pub const ACCOUNT_TYPES: [&str; 6] = ["wa", "tg", "gmail", "anthropic", "openai", "ollama"];

/// Tunnel types (the only ones the core accepts).
pub const TUNNEL_TYPES: [&str; 5] = ["wireguard", "tailscale", "ssh", "cloudflare", "ngrok"];

/// File size limit in the editor, characters.
pub const FILE_LIMIT: usize = 256 * 1024;

fn pairs(items: &[&str]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|v| (v.to_string(), v.to_string()))
        .collect()
}

pub fn account_new_specs() -> Vec<Spec> {
    vec![
        entity(
            "a:new:type",
            "account_new",
            "type",
            tr("set-f-acc-type"),
            Ty::Choice(pairs(&ACCOUNT_TYPES)),
            "wa",
        ),
        entity(
            "a:new:label",
            "account_new",
            "label",
            tr("set-f-acc-label"),
            Ty::Text(80),
            "",
        ),
    ]
}

pub fn mcp_new_specs() -> Vec<Spec> {
    vec![
        entity(
            "c:new:name",
            "mcp_new",
            "name",
            tr("set-f-mcp-name"),
            Ty::Text(32),
            "",
        ),
        entity(
            "c:new:url",
            "mcp_new",
            "url",
            tr("set-f-mcp-url"),
            Ty::Text(2048),
            "",
        ),
    ]
}

pub fn tunnel_new_specs() -> Vec<Spec> {
    vec![
        entity(
            "t:new:type",
            "tunnel_new",
            "type",
            tr("set-f-tun-type"),
            Ty::Choice(pairs(&TUNNEL_TYPES)),
            "wireguard",
        ),
        entity(
            "t:new:addr",
            "tunnel_new",
            "addr",
            tr("set-f-tun-addr"),
            Ty::Text(255),
            "",
        ),
        entity(
            "t:new:descr",
            "tunnel_new",
            "descr",
            tr("set-f-tun-descr"),
            Ty::Text(200),
            "",
        ),
    ]
}

pub fn module_url_spec() -> Spec {
    entity(
        "m:url",
        "module_new",
        "url",
        tr("set-f-module-url"),
        Ty::Text(2048),
        "",
    )
}

pub fn file_text_spec() -> Spec {
    entity(
        "f:text",
        "file",
        "text",
        tr("set-f-file-text"),
        Ty::Text(FILE_LIMIT),
        "",
    )
}
