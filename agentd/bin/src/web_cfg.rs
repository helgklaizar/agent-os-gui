//! Browser client settings from the `serve` flags (ADR-0010): static directory, own hosts and origin, dev CORS.

use std::path::PathBuf;

use agentd_api::ServerConfig;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WebOpts {
    /// `--web-root`: directory of the built client; without it static files are not served.
    pub root: Option<PathBuf>,
    /// `--allow-host`: additional allowed `Host` values (address of a public tunnel).
    pub allow_hosts: Vec<String>,
    /// `--allow-origin`: allowed `Origin` values (public tunnel address with scheme).
    pub allow_origins: Vec<String>,
    /// `--dev-cors`: dev server origin (development only).
    pub dev_cors: Vec<String>,
}

impl WebOpts {
    /// Moves the settings into the server configuration.
    pub fn apply(&self, cfg: &mut ServerConfig) {
        cfg.web_root.clone_from(&self.root);
        cfg.allowed_hosts.extend(self.allow_hosts.iter().cloned());
        cfg.allowed_origins
            .extend(self.allow_origins.iter().cloned());
        cfg.dev_cors.extend(self.dev_cors.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_moves_every_field_into_server_config() {
        let mut cfg = ServerConfig::new("/tmp/x.sock");
        cfg.allowed_hosts.push("old".into());
        let w = WebOpts {
            root: Some(PathBuf::from("/srv/web")),
            allow_hosts: vec!["h.example".into()],
            allow_origins: vec!["https://h.example".into()],
            dev_cors: vec!["http://localhost:5173".into()],
        };
        w.apply(&mut cfg);
        assert_eq!(cfg.web_root, Some(PathBuf::from("/srv/web")));
        assert_eq!(
            cfg.allowed_hosts,
            vec!["old".to_string(), "h.example".to_string()]
        );
        assert_eq!(cfg.allowed_origins, vec!["https://h.example".to_string()]);
        assert_eq!(cfg.dev_cors, vec!["http://localhost:5173".to_string()]);
        let mut empty = ServerConfig::new("/tmp/y.sock");
        WebOpts::default().apply(&mut empty);
        assert_eq!(empty.web_root, None);
        assert!(empty.dev_cors.is_empty());
    }
}
