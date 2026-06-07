//! Site settings (the auto-print toggle) and a health/diagnostics endpoint.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

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
    }))
}
