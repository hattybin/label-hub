//! Environment-driven configuration. The `.env` file does most of the lifting,
//! per the project design — there are intentionally very few knobs.
//!
//! The hub runs **two** HTTP listeners:
//!   * the *public* listener exposes only the D365 webhook and is bound to
//!     loopback by default (a tunnel sidecar on the same host forwards to it);
//!   * the *local* listener serves the console + management APIs to the LAN and
//!     is intended to be reached via mDNS (e.g. `printlabels.local`).

use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    // ── Public listener (D365 webhook only) ──────────────────────────────────
    /// Address the public/webhook listener binds to. Loopback by default so only
    /// a tunnel sidecar running on this host can reach it.
    pub public_bind: String,
    /// Port the public/webhook listener binds to.
    pub public_port: u16,
    /// The externally visible base URL (tunnel/relay host), shown in the console's
    /// D365 mapping guide. Display-only.
    pub public_url: Option<String>,

    // ── Local listener (console + management) ────────────────────────────────
    /// Address the local console listener binds to. LAN-wide by default.
    pub local_bind: String,
    /// Port the local console listener binds to.
    pub local_port: u16,

    // ── mDNS / discovery ─────────────────────────────────────────────────────
    /// Advertise the local console over mDNS so it's reachable by name.
    pub mdns_enable: bool,
    /// Hostname to advertise (without `.local`). `printlabels` → `printlabels.local`.
    pub mdns_hostname: String,

    // ── Behaviour ────────────────────────────────────────────────────────────
    /// Shared secret D365 must present (Authorization: Bearer ...). D365 `$auth.secret$`.
    pub inbound_secret: String,
    /// Human-readable site label shown in the console (e.g. "PLANT1").
    pub site_name: String,
    /// Optional fallback printer name used when an inbound job omits `X-Printer-Name`.
    pub default_printer: Option<String>,
    /// Default for the auto-print toggle when no persisted setting exists.
    pub auto_print_default: bool,
    /// Directory for JSON persistence (printers/jobs/settings).
    pub data_dir: String,

    // ── Optional central control plane (C2) ──────────────────────────────────
    /// Base URL of the control plane. When set, the node enrolls and syncs config.
    /// When unset, the node runs fully standalone from `.env` (Phase-1 behaviour).
    pub control_url: Option<String>,
    /// One-time enrollment token from the C2 dashboard (used only until enrolled).
    pub enrollment_token: Option<String>,
    /// Seconds between heartbeats to the control plane.
    pub heartbeat_secs: u64,
    /// Hostname reported to the C2 (defaults to the OS hostname).
    pub node_hostname: String,

    // ── Optional D365 OData (Entra app) ──────────────────────────────────────
    pub azure_tenant_id: Option<String>,
    pub azure_client_id: Option<String>,
    pub azure_client_secret: Option<String>,
    pub d365_base_url: Option<String>,
    pub d365_company: Option<String>,
}

fn opt(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

fn port(key: &str, default: u16) -> u16 {
    opt(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn flag(key: &str, default: bool) -> bool {
    opt(key)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

/// Best-effort read of the OS hostname (Linux/Unix).
fn read_hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl Config {
    /// True when the node should enroll with and sync from a control plane.
    pub fn control_enabled(&self) -> bool {
        self.control_url.is_some()
    }
}

impl Config {
    pub fn from_env() -> Self {
        // `PORT` is honoured as a back-compat alias for the public port.
        let public_port = opt("PUBLIC_PORT")
            .or_else(|| opt("PORT"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);

        let inbound_secret = opt("INBOUND_SECRET").unwrap_or_else(|| {
            tracing::warn!("INBOUND_SECRET is not set — the inbound webhook will reject all requests");
            String::new()
        });

        Config {
            public_bind: opt("PUBLIC_BIND").unwrap_or_else(|| "127.0.0.1".to_string()),
            public_port,
            public_url: opt("PUBLIC_URL").map(|v| v.trim_end_matches('/').to_string()),

            local_bind: opt("LOCAL_BIND").unwrap_or_else(|| "0.0.0.0".to_string()),
            local_port: port("LOCAL_PORT", 8081),

            mdns_enable: flag("MDNS_ENABLE", false),
            mdns_hostname: opt("MDNS_HOSTNAME").unwrap_or_else(|| "printlabels".to_string()),

            inbound_secret,
            site_name: opt("SITE_NAME").unwrap_or_else(|| "LABEL-HUB".to_string()),
            default_printer: opt("DEFAULT_PRINTER"),
            auto_print_default: flag("AUTO_PRINT", false),
            data_dir: opt("DATA_DIR").unwrap_or_else(|| "data".to_string()),

            control_url: opt("CONTROL_URL").map(|v| v.trim_end_matches('/').to_string()),
            enrollment_token: opt("ENROLLMENT_TOKEN"),
            heartbeat_secs: opt("HEARTBEAT_SECS").and_then(|v| v.parse().ok()).unwrap_or(30),
            node_hostname: opt("NODE_HOSTNAME")
                .or_else(|| opt("HOSTNAME"))
                .or_else(read_hostname)
                .unwrap_or_else(|| "label-hub".to_string()),

            azure_tenant_id: opt("AZURE_TENANT_ID"),
            azure_client_id: opt("AZURE_CLIENT_ID"),
            azure_client_secret: opt("AZURE_CLIENT_SECRET"),
            d365_base_url: opt("D365_BASE_URL").map(|v| v.trim_end_matches('/').to_string()),
            d365_company: opt("D365_COMPANY"),
        }
    }

    /// True when all credentials needed for the optional D365 OData client are present.
    pub fn d365_enabled(&self) -> bool {
        self.azure_tenant_id.is_some()
            && self.azure_client_id.is_some()
            && self.azure_client_secret.is_some()
            && self.d365_base_url.is_some()
    }

    /// The `.local` hostname advertised over mDNS, e.g. `printlabels.local`.
    pub fn mdns_fqdn(&self) -> String {
        format!("{}.local", self.mdns_hostname.trim_end_matches(".local"))
    }
}
