//! Node-facing API (tailnet-only in production). Per-node bearer token auth.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use label_proto::{Heartbeat, NodeConfig, Printer, RegisterRequest, RegisterResponse, Settings};
use serde_json::json;
use sqlx::types::Json as SqlxJson;

use crate::state::AppState;
use crate::util::random_token;

/// POST /api/enroll
pub async fn enroll(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    // Validate one-time enrollment token.
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT site FROM enrollment_tokens WHERE token = $1 AND used_at IS NULL",
    )
    .bind(&req.enrollment_token)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let Some((token_site,)) = row else {
        return (StatusCode::UNAUTHORIZED, "invalid or used enrollment token").into_response();
    };

    let site = req.site_hint.clone().unwrap_or(token_site);
    let node_id = uuid::Uuid::new_v4().to_string();
    let node_token = random_token();
    let inbound_secret = random_token();

    // Create node + initial config (version 1).
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    };

    let insert_node = sqlx::query(
        "INSERT INTO nodes (id, site, hostname, mgmt_port, app_version, node_token) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&node_id)
    .bind(&site)
    .bind(&req.hostname)
    .bind(req.mgmt_port as i32)
    .bind(req.app_version.clone().unwrap_or_default())
    .bind(&node_token)
    .execute(&mut *tx)
    .await;
    if let Err(e) = insert_node {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("insert node: {e}")).into_response();
    }

    let insert_cfg = sqlx::query(
        "INSERT INTO node_config (node_id, version, printers, settings, inbound_secret, public_url) \
         VALUES ($1, 1, '[]'::jsonb, $2, $3, NULL)",
    )
    .bind(&node_id)
    .bind(SqlxJson(Settings::default()))
    .bind(&inbound_secret)
    .execute(&mut *tx)
    .await;
    if let Err(e) = insert_cfg {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("insert cfg: {e}")).into_response();
    }

    let _ = sqlx::query(
        "UPDATE enrollment_tokens SET used_at = now(), used_by_node = $1 WHERE token = $2",
    )
    .bind(&node_id)
    .bind(&req.enrollment_token)
    .execute(&mut *tx)
    .await;

    if let Err(e) = tx.commit().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("commit: {e}")).into_response();
    }

    let tailscale_authkey = crate::tailscale::mint_authkey(&state).await;

    let config = NodeConfig {
        version: 1,
        printers: Vec::new(),
        settings: Settings::default(),
        inbound_secret,
        public_url: None,
    };

    tracing::info!("enrolled node {} (site {})", node_id, site);
    Json(RegisterResponse {
        node_id,
        node_token,
        tailscale_authkey,
        config,
    })
    .into_response()
}

/// POST /api/nodes/:id/heartbeat
pub async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(hb): Json<Heartbeat>,
) -> impl IntoResponse {
    if let Err(code) = authorize(&state, &id, &headers).await {
        return code.into_response();
    }
    let res = sqlx::query(
        "UPDATE nodes SET last_seen = now(), app_version = $2, config_version = $3, \
         queue_depth = $4, hostname = COALESCE($5, hostname), mgmt_port = $6, printers_json = $7 \
         WHERE id = $1",
    )
    .bind(&id)
    .bind(&hb.app_version)
    .bind(hb.config_version as i64)
    .bind(hb.queue_depth as i32)
    .bind(hb.hostname.clone())
    .bind(hb.mgmt_port as i32)
    .bind(SqlxJson(&hb.printers))
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    }
}

/// GET /api/nodes/:id/config
pub async fn get_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = authorize(&state, &id, &headers).await {
        return code.into_response();
    }
    match load_config(&state, &id).await {
        Some(cfg) => Json(cfg).into_response(),
        None => (StatusCode::NOT_FOUND, "no config").into_response(),
    }
}

/// POST /api/nodes/:id/events  — body: [PrintEvent]
pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(events): Json<Vec<label_proto::PrintEvent>>,
) -> impl IntoResponse {
    if let Err(code) = authorize(&state, &id, &headers).await {
        return code.into_response();
    }
    for ev in events {
        let _ = sqlx::query(
            "INSERT INTO print_events (node_id, printer, status, source, error) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(&id)
        .bind(&ev.printer)
        .bind(&ev.status)
        .bind(&ev.source)
        .bind(&ev.error)
        .execute(&state.db)
        .await;
    }
    Json(json!({ "ok": true })).into_response()
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn authorize(state: &AppState, id: &str, headers: &HeaderMap) -> Result<(), StatusCode> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v).trim().to_string())
        .unwrap_or_default();
    if presented.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let row: Option<(String,)> = sqlx::query_as("SELECT node_token FROM nodes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    match row {
        Some((tok,)) if constant_time_eq(tok.as_bytes(), presented.as_bytes()) => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn load_config(state: &AppState, id: &str) -> Option<NodeConfig> {
    let row: Option<(i64, SqlxJson<Vec<Printer>>, SqlxJson<Settings>, String, Option<String>)> =
        sqlx::query_as(
            "SELECT version, printers, settings, inbound_secret, public_url FROM node_config WHERE node_id = $1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    row.map(|(version, printers, settings, inbound_secret, public_url)| NodeConfig {
        version: version as u64,
        printers: printers.0,
        settings: settings.0,
        inbound_secret,
        public_url,
    })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
