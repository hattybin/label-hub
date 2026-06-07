//! Control-plane agent. When `CONTROL_URL` is set, the node enrolls once, then
//! heartbeats and pulls its desired config on a loop. Everything is best-effort:
//! if the control plane is unreachable the node keeps serving from cached config,
//! so printing never depends on the cloud.

use std::time::Duration;

use label_proto::{Heartbeat, NodeConfig, PrinterStatus, RegisterRequest, RegisterResponse};

use crate::printer;
use crate::state::{AppState, NodeCreds};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Background task: enroll (if needed) then heartbeat + sync forever.
pub async fn run(state: AppState) {
    let Some(control_url) = state.config.control_url.clone() else {
        return; // standalone mode
    };
    tracing::info!("control-plane agent enabled → {control_url}");

    let interval = Duration::from_secs(state.config.heartbeat_secs.max(5));

    loop {
        if state.get_creds().await.is_none() {
            if let Err(e) = enroll(&state, &control_url).await {
                tracing::warn!("enrollment failed (will retry): {e}");
                tokio::time::sleep(interval).await;
                continue;
            }
        }

        if let Err(e) = heartbeat_and_sync(&state, &control_url).await {
            tracing::debug!("control sync failed (serving cached config): {e}");
        }

        tokio::time::sleep(interval).await;
    }
}

async fn enroll(state: &AppState, control_url: &str) -> Result<(), String> {
    let token = state
        .config
        .enrollment_token
        .clone()
        .ok_or("ENROLLMENT_TOKEN not set")?;

    let req = RegisterRequest {
        enrollment_token: token,
        hostname: state.config.node_hostname.clone(),
        site_hint: Some(state.config.site_name.clone()),
        app_version: Some(APP_VERSION.to_string()),
        mgmt_port: state.config.local_port,
    };

    let resp = state
        .http
        .post(format!("{control_url}/api/enroll"))
        .json(&req)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!(
            "enroll HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }

    let rr: RegisterResponse = resp.json().await.map_err(|e| e.to_string())?;
    state
        .set_creds(NodeCreds {
            node_id: rr.node_id.clone(),
            node_token: rr.node_token.clone(),
        })
        .await;
    state.apply_config(&rr.config).await;

    if rr.tailscale_authkey.is_some() {
        // The app does not run `tailscale up` itself — the provisioning script /
        // first-boot service handles mesh join. We just note the key was issued.
        tracing::info!("control plane issued a Tailscale auth key (applied by provisioning)");
    }
    tracing::info!("enrolled as node {}", rr.node_id);
    Ok(())
}

async fn heartbeat_and_sync(state: &AppState, control_url: &str) -> Result<(), String> {
    let creds = state.get_creds().await.ok_or("not enrolled")?;

    // Build the heartbeat (probe printers concurrently with a short timeout).
    let (queue_depth, printers, last_print_at) = {
        let store = state.store.lock().await;
        let last = store
            .history
            .iter()
            .find(|j| j.printed_at.is_some())
            .and_then(|j| j.printed_at.clone());
        (store.pending.len() as u32, store.printers.clone(), last)
    };
    let probes = printers.iter().map(|p| {
        let ip = p.ip.clone();
        let port = p.port;
        let name = p.name.clone();
        async move {
            PrinterStatus {
                name,
                reachable: printer::is_reachable(&ip, port).await,
            }
        }
    });
    let printer_status = futures::future::join_all(probes).await;

    let hb = Heartbeat {
        app_version: APP_VERSION.to_string(),
        config_version: state.config_version().await,
        queue_depth,
        hostname: Some(state.config.node_hostname.clone()),
        mgmt_port: state.config.local_port,
        printers: printer_status,
        last_print_at,
        recent_errors: Vec::new(),
    };

    let hb_resp = state
        .http
        .post(format!("{control_url}/api/nodes/{}/heartbeat", creds.node_id))
        .bearer_auth(&creds.node_token)
        .json(&hb)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if hb_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("heartbeat unauthorized (node may have been removed)".to_string());
    }

    pull_config(state, control_url, &creds).await.map(|_| ())
}

/// Fetch the desired config and apply it if newer. Returns true if a new config
/// was applied. Also used by the `/api/admin/refresh` route for immediate pulls.
pub async fn pull_config(
    state: &AppState,
    control_url: &str,
    creds: &NodeCreds,
) -> Result<bool, String> {
    let resp = state
        .http
        .get(format!("{control_url}/api/nodes/{}/config", creds.node_id))
        .bearer_auth(&creds.node_token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("config HTTP {}", resp.status()));
    }
    let cfg: NodeConfig = resp.json().await.map_err(|e| e.to_string())?;
    Ok(state.apply_config(&cfg).await)
}

/// Fire-and-forget report of a print/audit event to the control plane. No-op in
/// standalone mode or before enrollment.
pub fn report_event(state: &AppState, ev: label_proto::PrintEvent) {
    let Some(control_url) = state.config.control_url.clone() else {
        return;
    };
    let state = state.clone();
    tokio::spawn(async move {
        if let Some(creds) = state.get_creds().await {
            let _ = state
                .http
                .post(format!("{control_url}/api/nodes/{}/events", creds.node_id))
                .bearer_auth(&creds.node_token)
                .json(&vec![ev])
                .send()
                .await;
        }
    });
}

/// Trigger an immediate config pull (called by `POST /api/admin/refresh`).
pub async fn refresh_now(state: &AppState) -> Result<bool, String> {
    let control_url = state.config.control_url.clone().ok_or("control plane not configured")?;
    let creds = state.get_creds().await.ok_or("not enrolled")?;
    pull_config(state, &control_url, &creds).await
}
