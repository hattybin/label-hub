//! Site settings (the auto-print toggle), a health/diagnostics endpoint,
//! and a .env read/write API so operators can edit config from the UI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use crate::state::AppState;

// ── .env editor ──────────────────────────────────────────────────────────────

const SECRET_KEYS: &[&str] = &["INBOUND_SECRET", "AZURE_CLIENT_SECRET"];

const EDITABLE_KEYS: &[&str] = &[
    // Site
    "SITE_NAME", "PUBLIC_URL", "INBOUND_SECRET", "DEFAULT_PRINTER",
    // Network (restart required)
    "MDNS_ENABLE", "MDNS_HOSTNAME", "LOCAL_PORT", "PUBLIC_PORT",
    // D365
    "AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET",
    "D365_BASE_URL", "D365_COMPANY",
    // D365 entity overrides
    "D365_RECEIPT_HEADER_ENTITY", "D365_RECEIPT_LINES_ENTITY", "D365_RECEIPT_DATE_FIELD",
];

fn env_path() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join(".env")
}

fn read_env_map() -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(env_path()) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

/// GET /api/admin/env — current .env values; secrets masked as "***".
pub async fn get_env(_: State<AppState>) -> impl IntoResponse {
    let map = read_env_map();
    let mut out = serde_json::Map::new();
    for key in EDITABLE_KEYS {
        let val = map.get(*key).cloned().unwrap_or_default();
        let display = if SECRET_KEYS.contains(key) && !val.is_empty() {
            "***".to_string()
        } else {
            val
        };
        out.insert(key.to_string(), json!(display));
    }
    Json(serde_json::Value::Object(out))
}

/// POST /api/admin/env — write updated key-value pairs to .env.
/// Preserves comments and line order. Sending "***" for a secret key keeps
/// the existing value unchanged.
pub async fn put_env(
    State(_state): State<AppState>,
    Json(input): Json<HashMap<String, String>>,
) -> impl IntoResponse {
    let path = env_path();
    let existing_raw = std::fs::read_to_string(&path).unwrap_or_default();

    let updates: HashMap<String, String> = input
        .into_iter()
        .filter(|(k, v)| {
            EDITABLE_KEYS.contains(&k.as_str())
                && !(SECRET_KEYS.contains(&k.as_str()) && v == "***")
        })
        .collect();

    let mut updated_keys: HashSet<String> = HashSet::new();
    let mut new_lines: Vec<String> = Vec::new();

    for line in existing_raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            new_lines.push(line.to_string());
            continue;
        }
        if let Some((k, _)) = trimmed.split_once('=') {
            let k = k.trim();
            if let Some(new_val) = updates.get(k) {
                new_lines.push(format!("{k}={new_val}"));
                updated_keys.insert(k.to_string());
                continue;
            }
        }
        new_lines.push(line.to_string());
    }

    // Append any keys that weren't already in the file
    for (k, v) in &updates {
        if !updated_keys.contains(k) {
            new_lines.push(format!("{k}={v}"));
        }
    }

    let content = new_lines.join("\n") + "\n";
    let tmp = path.with_extension("tmp");
    if let Err(e) =
        std::fs::write(&tmp, &content).and_then(|_| std::fs::rename(&tmp, &path))
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response();
    }

    Json(json!({ "ok": true, "message": "Saved — restart the service to apply changes." }))
        .into_response()
}

/// POST /api/admin/restart — restart the service without downloading a new binary.
/// Requires sudoers: labelhub ALL=(root) NOPASSWD: /usr/bin/systemctl restart label-hub
pub async fn restart(State(state): State<AppState>) -> impl IntoResponse {
    if state.update_running.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "update already in progress" })),
        )
            .into_response();
    }
    let flag = Arc::clone(&state.update_running);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        match Command::new("sudo")
            .args(["systemctl", "restart", "label-hub"])
            .output()
            .await
        {
            Ok(_) => flag.store(false, Ordering::SeqCst),
            Err(e) => {
                tracing::error!("restart failed: {e}");
                flag.store(false, Ordering::SeqCst);
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({ "ok": true, "message": "Service restarting in ~2 s" })),
    )
        .into_response()
}

/// POST /api/admin/refresh — trigger an immediate control-plane config pull.
/// Called by the C2 over the mesh after editing a node's config.
pub async fn refresh(State(state): State<AppState>) -> impl IntoResponse {
    match crate::agent::refresh_now(&state).await {
        Ok(applied) => Json(json!({ "ok": true, "applied": applied })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

/// GET /api/settings
pub async fn get(State(state): State<AppState>) -> impl IntoResponse {
    let store = state.store.lock().await;
    Json(store.settings.clone())
}

#[derive(Deserialize)]
pub struct SettingsInput {
    pub auto_print: Option<bool>,
}

/// PUT /api/settings — partial update.
pub async fn put(
    State(state): State<AppState>,
    Json(input): Json<SettingsInput>,
) -> impl IntoResponse {
    {
        let mut store = state.store.lock().await;
        if let Some(v) = input.auto_print {
            store.settings.auto_print = v;
        }
    }
    state.save_settings().await;
    let settings = { state.store.lock().await.settings.clone() };
    state.broadcast(&json!({ "type": "settings", "settings": settings }));
    Json(settings)
}

async fn svc_active(name: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", name])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn tailscale_connected() -> bool {
    Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .await
        .map(|o| {
            if !o.status.success() { return false; }
            let body = String::from_utf8_lossy(&o.stdout);
            body.contains("\"BackendState\":\"Running\"")
        })
        .unwrap_or(false)
}

/// GET /api/health — config snapshot for the Site Management tab.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.config;
    let (pending, history, printers, auto_print, secret_set, public_url, config_version) = {
        let store = state.store.lock().await;
        (
            store.pending.len(),
            store.history.len(),
            store.printers.len(),
            store.settings.auto_print,
            !store.inbound_secret.is_empty(),
            store.public_url.clone(),
            store.config_version,
        )
    };
    let creds = state.get_creds().await;
    let (azbridge_up, tailscale_up) = tokio::join!(
        svc_active("azbridge"),
        tailscale_connected(),
    );
    Json(json!({
        "status": "ok",
        "site": cfg.site_name,
        "secretConfigured": secret_set,
        "defaultPrinter": cfg.default_printer,
        "autoPrint": auto_print,
        "counts": { "pending": pending, "history": history, "printers": printers },
        "d365": {
            "enabled": cfg.d365_enabled(),
            "baseUrl": cfg.d365_base_url,
            "company": cfg.d365_company,
        },
        // The public-facing host D365 must call; served on the LOCAL listener, so we
        // cannot infer it from the URL — comes from .env or control-plane config.
        "publicUrl": public_url,
        "inboundPath": "/api/print/inbound",
        "listeners": {
            "publicPort": cfg.public_port,
            "localPort": cfg.local_port,
        },
        "mdns": {
            "enabled": cfg.mdns_enable,
            "host": if cfg.mdns_enable { Some(cfg.mdns_fqdn()) } else { None },
        },
        "control": {
            "enabled": cfg.control_enabled(),
            "url": cfg.control_url,
            "enrolled": creds.is_some(),
            "nodeId": creds.as_ref().map(|c| c.node_id.clone()),
            "configVersion": config_version,
        },
        "services": {
            "azbridge": if azbridge_up { "connected" } else { "offline" },
            "tailscale": if tailscale_up { "connected" } else { "offline" },
        },
    }))
}

/// POST /api/admin/update — pull and install the latest binary from GitHub, then
/// restart the service.  Returns 202 immediately; the update runs in the background.
/// The service will restart ~3 s after the response arrives.
///
/// Requires a sudoers entry on the Pi:
///   labelhub ALL=(root) NOPASSWD: /opt/label-hub-src/deploy/update.sh
pub async fn update(State(state): State<AppState>) -> impl IntoResponse {
    if state.update_running.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "update already in progress" })),
        )
            .into_response();
    }

    let flag = Arc::clone(&state.update_running);
    tokio::spawn(async move {
        tracing::info!("update: spawning /opt/label-hub-src/deploy/update.sh");
        match Command::new("sudo")
            .args(["/opt/label-hub-src/deploy/update.sh"])
            .output()
            .await
        {
            Ok(out) => {
                // Successful update ends in `systemctl restart`, which kills this
                // process before output() returns — we only reach here on failure.
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::error!(
                    "update script exited {:?} (expected restart killed us first)\nstdout: {stdout}\nstderr: {stderr}",
                    out.status.code()
                );
                flag.store(false, Ordering::SeqCst);
            }
            Err(e) => {
                tracing::error!("update: failed to spawn script: {e}");
                flag.store(false, Ordering::SeqCst);
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "ok": true, "message": "update triggered; service will restart in ~5 s" })),
    )
        .into_response()
}
