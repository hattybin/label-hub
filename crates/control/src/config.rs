//! Control-plane configuration from environment.

use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,

    /// Node-facing API (tailnet-only in production): enroll/heartbeat/config/events.
    pub node_api_bind: String,
    pub node_api_port: u16,

    /// Dashboard (Entra SSO via EasyAuth in production).
    pub dash_bind: String,
    pub dash_port: u16,

    /// How the control plane reaches nodes: `{scheme}://{hostname}:{mgmt_port}`.
    pub node_scheme: String,

    /// Optional Tailscale OAuth client for minting per-node auth keys.
    pub ts_oauth_client_id: Option<String>,
    pub ts_oauth_client_secret: Option<String>,
    pub ts_tailnet: Option<String>,
    pub ts_tag: String,

    /// Dev convenience: when set (an email), requests without an EasyAuth principal
    /// are treated as this admin. NEVER set in production.
    pub dev_admin: Option<String>,
}

fn opt(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            database_url: opt("DATABASE_URL")
                .expect("DATABASE_URL is required (e.g. postgres://user:pass@host/db)"),
            node_api_bind: opt("NODE_API_BIND").unwrap_or_else(|| "0.0.0.0".into()),
            node_api_port: opt("NODE_API_PORT").and_then(|v| v.parse().ok()).unwrap_or(9090),
            dash_bind: opt("DASH_BIND").unwrap_or_else(|| "0.0.0.0".into()),
            dash_port: opt("DASH_PORT").and_then(|v| v.parse().ok()).unwrap_or(9091),
            node_scheme: opt("NODE_SCHEME").unwrap_or_else(|| "http".into()),
            ts_oauth_client_id: opt("TS_OAUTH_CLIENT_ID"),
            ts_oauth_client_secret: opt("TS_OAUTH_CLIENT_SECRET"),
            ts_tailnet: opt("TS_TAILNET"),
            ts_tag: opt("TS_TAG").unwrap_or_else(|| "tag:lh-node".into()),
            dev_admin: opt("DEV_ADMIN"),
        }
    }

    pub fn tailscale_enabled(&self) -> bool {
        self.ts_oauth_client_id.is_some()
            && self.ts_oauth_client_secret.is_some()
            && self.ts_tailnet.is_some()
    }
}
