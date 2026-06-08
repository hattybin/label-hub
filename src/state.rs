//! Shared application state: printer profiles, the job queue + history, site
//! settings, and an SSE broadcast channel. Mirrors the JSON-file persistence
//! model of the original Node prototype (printers.json / queue.json).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::config::Config;

// Shared wire types live in the `label-proto` crate so the node and control plane
// agree on the contract. Re-exported here so existing `state::Printer` /
// `state::Settings` references keep working.
pub use label_proto::{Printer, Settings};

pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Printed,
    Failed,
    Dismissed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// Printer name as supplied by D365 (`$label.printer$`), resolved against profiles.
    pub printer: String,
    /// Raw ZPL to send to the printer.
    pub zpl: String,
    /// Where the job came from: "d365" (webhook) or "manual".
    pub source: String,
    pub received_at: String,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub printed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Best-effort count of labels in the ZPL (number of ^XA blocks), for display.
    pub label_count: u32,
}

// ── Persisted document shapes ────────────────────────────────────────────────

#[derive(Default, Serialize, Deserialize)]
struct PrintersDoc {
    printers: Vec<Printer>,
}

#[derive(Default, Serialize, Deserialize)]
struct JobsDoc {
    #[serde(default)]
    pending: Vec<Job>,
    #[serde(default)]
    history: Vec<Job>,
}

/// Persisted node credentials (assigned by the control plane at enrollment).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCreds {
    pub node_id: String,
    pub node_token: String,
}

/// In-memory store, flushed to disk on each mutation.
pub struct Store {
    pub printers: Vec<Printer>,
    pub pending: Vec<Job>,
    pub history: Vec<Job>,
    pub settings: Settings,
    // Runtime-mutable config — seeded from `.env`, overridden by control-plane
    // config when enrolled. Kept here (not in the immutable Config) so the C2 can
    // rotate the secret / change the public URL without a restart.
    pub inbound_secret: String,
    pub public_url: Option<String>,
    pub config_version: u64,
}

const MAX_PENDING: usize = 500;
const MAX_HISTORY: usize = 1000;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Arc<Mutex<Store>>,
    pub tx: broadcast::Sender<String>,
    pub http: reqwest::Client,
    /// Cached D365 OData bearer token (token, expires_at_unix_secs).
    pub d365_token: Arc<Mutex<Option<(String, i64)>>>,
    /// Cached D365 entity name resolution: key ("receiptHeaders" / "receiptLines") → entity name.
    pub entity_cache: Arc<RwLock<HashMap<String, String>>>,
    /// Cached receipt date field per entity name: entity_name → field name (None = not discoverable).
    pub date_field_cache: Arc<RwLock<HashMap<String, Option<String>>>>,
    /// Control-plane credentials, once enrolled.
    pub creds: Arc<Mutex<Option<NodeCreds>>>,
    /// Set while a self-update is in progress; prevents concurrent triggers.
    pub update_running: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let data_dir = PathBuf::from(&config.data_dir);
        let _ = std::fs::create_dir_all(&data_dir);

        let printers = read_json::<PrintersDoc>(&data_dir.join("printers.json"))
            .unwrap_or_default()
            .printers;
        let jobs = read_json::<JobsDoc>(&data_dir.join("jobs.json")).unwrap_or_default();
        let settings = read_json::<Settings>(&data_dir.join("settings.json"))
            .unwrap_or(Settings { auto_print: config.auto_print_default });

        // Seed runtime config from .env, then let a cached control-plane config
        // (if present) take precedence so the node restarts with last-known state.
        let mut store = Store {
            printers,
            pending: jobs.pending,
            history: jobs.history,
            settings,
            inbound_secret: config.inbound_secret.clone(),
            public_url: config.public_url.clone(),
            config_version: 0,
        };
        if let Some(cached) = read_json::<label_proto::NodeConfig>(&data_dir.join("config.json")) {
            apply_config_to_store(&mut store, &cached);
            tracing::info!("loaded cached control-plane config v{}", cached.version);
        }

        let creds = read_json::<NodeCreds>(&data_dir.join("node.json"));

        let (tx, _rx) = broadcast::channel::<String>(256);

        AppState {
            config: Arc::new(config),
            store: Arc::new(Mutex::new(store)),
            tx,
            http: reqwest::Client::builder()
                .user_agent("label-hub")
                .build()
                .expect("build reqwest client"),
            d365_token: Arc::new(Mutex::new(None)),
            entity_cache: Arc::new(RwLock::new(HashMap::new())),
            date_field_cache: Arc::new(RwLock::new(HashMap::new())),
            creds: Arc::new(Mutex::new(creds)),
            update_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Broadcast a JSON event to all connected SSE clients.
    pub fn broadcast(&self, event: &serde_json::Value) {
        if let Ok(s) = serde_json::to_string(event) {
            let _ = self.tx.send(s);
        }
    }

    fn data_dir(&self) -> PathBuf {
        PathBuf::from(&self.config.data_dir)
    }

    pub async fn save_printers(&self) {
        let store = self.store.lock().await;
        write_json(
            &self.data_dir().join("printers.json"),
            &PrintersDoc { printers: store.printers.clone() },
        );
    }

    pub async fn save_jobs(&self) {
        let mut store = self.store.lock().await;
        store.pending.truncate(MAX_PENDING);
        store.history.truncate(MAX_HISTORY);
        let doc = JobsDoc {
            pending: store.pending.clone(),
            history: store.history.clone(),
        };
        write_json(&self.data_dir().join("jobs.json"), &doc);
    }

    pub async fn save_settings(&self) {
        let store = self.store.lock().await;
        write_json(&self.data_dir().join("settings.json"), &store.settings);
    }

    /// Current effective inbound secret (control-plane value when enrolled).
    pub async fn inbound_secret(&self) -> String {
        self.store.lock().await.inbound_secret.clone()
    }

    /// Apply a control-plane config: update runtime state, persist all caches, and
    /// notify the console. Only applies when the version is newer (or forced).
    pub async fn apply_config(&self, cfg: &label_proto::NodeConfig) -> bool {
        {
            let mut store = self.store.lock().await;
            if cfg.version <= store.config_version && store.config_version != 0 {
                return false; // not newer
            }
            apply_config_to_store(&mut store, cfg);
        }
        // Persist the cache + the individual files the console/handlers read.
        write_json(&self.data_dir().join("config.json"), cfg);
        self.save_printers().await;
        self.save_settings().await;
        let settings = { self.store.lock().await.settings.clone() };
        self.broadcast(&serde_json::json!({ "type": "settings", "settings": settings }));
        self.broadcast(&serde_json::json!({ "type": "config_applied", "version": cfg.version }));
        tracing::info!("applied control-plane config v{}", cfg.version);
        true
    }

    pub async fn config_version(&self) -> u64 {
        self.store.lock().await.config_version
    }

    pub async fn get_creds(&self) -> Option<NodeCreds> {
        self.creds.lock().await.clone()
    }

    pub async fn set_creds(&self, creds: NodeCreds) {
        write_json(&self.data_dir().join("node.json"), &creds);
        *self.creds.lock().await = Some(creds);
    }
}

/// Overwrite the runtime-mutable parts of the store from a control-plane config.
fn apply_config_to_store(store: &mut Store, cfg: &label_proto::NodeConfig) {
    store.printers = cfg.printers.clone();
    store.settings = cfg.settings.clone();
    if !cfg.inbound_secret.is_empty() {
        store.inbound_secret = cfg.inbound_secret.clone();
    }
    if cfg.public_url.is_some() {
        store.public_url = cfg.public_url.clone();
    }
    store.config_version = cfg.version;
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("could not parse {}: {}", path.display(), e);
            None
        }
    }
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => {
            // Write to a temp file then rename for atomicity.
            let tmp = path.with_extension("tmp");
            if let Err(e) = std::fs::write(&tmp, s).and_then(|_| std::fs::rename(&tmp, path)) {
                tracing::warn!("could not write {}: {}", path.display(), e);
            }
        }
        Err(e) => tracing::warn!("could not serialize {}: {}", path.display(), e),
    }
}

/// Count `^XA` label openings to estimate how many labels a ZPL payload contains.
pub fn count_labels(zpl: &str) -> u32 {
    let n = zpl.matches("^XA").count() as u32;
    n.max(1)
}
