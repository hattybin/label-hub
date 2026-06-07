//! Shared, I/O-free data types exchanged between the label-hub **node** and the
//! **control plane** (C2). Both sides depend on this crate so the wire contract is
//! defined once.

use serde::{Deserialize, Serialize};

// ── Printer profile ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub ip: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

pub fn default_port() -> u16 {
    9100
}

// ── Site/runtime settings ────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    /// When true, inbound jobs print on arrival; when false they are held.
    pub auto_print: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { auto_print: false }
    }
}

// ── Desired node configuration (control → node) ──────────────────────────────

/// The full desired state the control plane hands to a node. The node caches this
/// locally and keeps serving from the cache if the control plane is unreachable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Monotonic version; the node applies a config only when this increases.
    pub version: u64,
    #[serde(default)]
    pub printers: Vec<Printer>,
    #[serde(default)]
    pub settings: Settings,
    /// Shared secret D365 must present on the inbound webhook (`$auth.secret$`).
    #[serde(default)]
    pub inbound_secret: String,
    /// Public tunnel/relay host shown in the D365 mapping guide (display only).
    #[serde(default)]
    pub public_url: Option<String>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        NodeConfig {
            version: 0,
            printers: Vec::new(),
            settings: Settings::default(),
            inbound_secret: String::new(),
            public_url: None,
        }
    }
}

// ── Enrollment (node → control, once) ────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    /// One-time enrollment token issued by the C2 dashboard.
    pub enrollment_token: String,
    /// The node's hostname (also used as the MagicDNS name hint).
    pub hostname: String,
    /// Optional human site name hint (e.g. "PLANT1").
    #[serde(default)]
    pub site_hint: Option<String>,
    /// App version reporting.
    #[serde(default)]
    pub app_version: Option<String>,
    /// Port the node's management API listens on (the control plane reaches the
    /// node at `http://{hostname}:{mgmt_port}` over the mesh).
    #[serde(default = "default_mgmt_port")]
    pub mgmt_port: u16,
}

pub fn default_mgmt_port() -> u16 {
    8081
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    /// Stable node id assigned by the control plane.
    pub node_id: String,
    /// Long-lived per-node bearer token for subsequent calls.
    pub node_token: String,
    /// Optional Tailscale auth key minted for this node (tagged, pre-authorized).
    #[serde(default)]
    pub tailscale_authkey: Option<String>,
    /// Initial desired configuration.
    pub config: NodeConfig,
}

// ── Heartbeat (node → control, periodic) ─────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrinterStatus {
    pub name: String,
    pub reachable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Heartbeat {
    pub app_version: String,
    pub config_version: u64,
    pub queue_depth: u32,
    /// Hostname + mgmt port so the control plane always knows where to reach back.
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default = "default_mgmt_port")]
    pub mgmt_port: u16,
    #[serde(default)]
    pub printers: Vec<PrinterStatus>,
    #[serde(default)]
    pub last_print_at: Option<String>,
    #[serde(default)]
    pub recent_errors: Vec<String>,
}

// ── Audit / print events (node → control) ────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrintEvent {
    pub printer: String,
    /// "printed" | "failed" | "dismissed" | "queued".
    pub status: String,
    /// "d365" | "reprint" | "manual".
    pub source: String,
    pub at: String,
    #[serde(default)]
    pub error: Option<String>,
}
