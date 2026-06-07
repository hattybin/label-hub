//! Dashboard API (Entra SSO via EasyAuth in production; `DEV_ADMIN` for local dev).
//! RBAC: admins see all sites; operators see only assigned sites.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use label_proto::{Printer, Settings};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::types::Json as SqlxJson;

use crate::auth::{allowed_sites, principal, Principal};
use crate::routes::node_api::load_config;
use crate::state::AppState;
use crate::util::random_token;

fn require(state: &AppState, headers: &HeaderMap) -> Result<Principal, StatusCode> {
    principal(state, headers).ok_or(StatusCode::UNAUTHORIZED)
}

/// GET /dash/me
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match require(&state, &headers) {
        Ok(p) => Json(json!({ "email": p.email, "roles": p.roles, "admin": p.is_admin() })).into_response(),
        Err(c) => c.into_response(),
    }
}

/// GET /dash/nodes
pub async fn list_nodes(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    let sites = allowed_sites(&state, &p).await;

    let rows: Vec<NodeRow> = match &sites {
        None => sqlx::query_as::<_, NodeRow>(NODE_SELECT)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default(),
        Some(list) => sqlx::query_as::<_, NodeRow>(&format!("{NODE_SELECT} WHERE n.site = ANY($1)"))
            .bind(list)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default(),
    };
    Json(rows.into_iter().map(NodeRow::to_json).collect::<Vec<_>>()).into_response()
}

/// GET /dash/nodes/:id
pub async fn get_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    let row: Option<NodeRow> = sqlx::query_as::<_, NodeRow>(&format!("{NODE_SELECT} WHERE n.id = $1"))
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "node not found").into_response();
    };
    if !can_access(&state, &p, &row.site).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cfg = load_config(&state, &id).await;
    Json(json!({ "node": row.to_json(), "config": cfg })).into_response()
}

#[derive(Deserialize)]
pub struct ConfigUpdate {
    pub printers: Vec<Printer>,
    pub settings: Settings,
    /// "rotate" → new random secret; "keep" → unchanged; otherwise set verbatim.
    #[serde(default)]
    pub inbound_secret: Option<String>,
    #[serde(default)]
    pub public_url: Option<String>,
}

/// PUT /dash/nodes/:id/config
pub async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(upd): Json<ConfigUpdate>,
) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    let site = match site_of(&state, &id).await {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "node not found").into_response(),
    };
    if !can_access(&state, &p, &site).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Resolve the secret directive.
    let new_secret = match upd.inbound_secret.as_deref() {
        Some("rotate") => Some(random_token()),
        Some("keep") | None => None,
        Some(s) => Some(s.to_string()),
    };

    let res = sqlx::query(
        "UPDATE node_config SET version = version + 1, printers = $2, settings = $3, \
         public_url = $4, inbound_secret = COALESCE($5, inbound_secret), updated_at = now() \
         WHERE node_id = $1",
    )
    .bind(&id)
    .bind(SqlxJson(&upd.printers))
    .bind(SqlxJson(&upd.settings))
    .bind(&upd.public_url)
    .bind(&new_secret)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response();
    }

    audit(&state, &p.email, "update_config", &id, json!({"rotated": new_secret.is_some()})).await;

    // Best-effort immediate push; the node also pulls on its next heartbeat.
    let pushed = push_refresh(&state, &id).await;

    Json(json!({ "ok": true, "pushed": pushed })).into_response()
}

// ── Enrollment tokens ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NewToken {
    pub site: String,
    #[serde(default)]
    pub note: String,
}

/// POST /dash/enrollment-tokens
pub async fn create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NewToken>,
) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    if !p.is_admin() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let token = random_token();
    let res = sqlx::query("INSERT INTO enrollment_tokens (token, site, note) VALUES ($1,$2,$3)")
        .bind(&token)
        .bind(&req.site)
        .bind(&req.note)
        .execute(&state.db)
        .await;
    if let Err(e) = res {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response();
    }
    audit(&state, &p.email, "create_token", &req.site, json!({})).await;
    Json(json!({ "token": token, "site": req.site })).into_response()
}

/// GET /dash/enrollment-tokens
pub async fn list_tokens(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    if !p.is_admin() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT token, site, note, used_by_node FROM enrollment_tokens ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    Json(
        rows.into_iter()
            .map(|(token, site, note, used)| json!({
                "token": token, "site": site, "note": note, "used": used.is_some(), "usedBy": used
            }))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

/// GET /dash/nodes/:id/events
pub async fn node_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if require(&state, &headers).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rows: Vec<(String, String, String, Option<String>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT printer, status, source, error, at FROM print_events WHERE node_id = $1 ORDER BY at DESC LIMIT 200",
        )
        .bind(&id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Json(
        rows.into_iter()
            .map(|(printer, status, source, error, at)| json!({
                "printer": printer, "status": status, "source": source, "error": error,
                "at": at.to_rfc3339()
            }))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

/// POST /dash/nodes/:id/test-print  — body: { "printer": "NAME" }
/// The control plane knows the node's inbound secret, so it sends a test label
/// through the node's normal inbound path over the mesh.
pub async fn test_print(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let p = match require(&state, &headers) {
        Ok(p) => p,
        Err(c) => return c.into_response(),
    };
    let site = match site_of(&state, &id).await {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "node not found").into_response(),
    };
    if !can_access(&state, &p, &site).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    let printer = body.get("printer").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if printer.is_empty() {
        return (StatusCode::BAD_REQUEST, "printer required").into_response();
    }
    let Some((host, port)) = node_addr(&state, &id).await else {
        return (StatusCode::NOT_FOUND, "node address unknown").into_response();
    };
    let cfg = load_config(&state, &id).await;
    let secret = cfg.map(|c| c.inbound_secret).unwrap_or_default();
    let zpl = "^XA^FO40,40^A0N,40,40^FDLabel Hub test^FS^FO40,100^A0N,28,28^FDControl plane^FS^XZ";

    let resp = state
        .http
        .post(format!("{}/api/print/inbound", state.node_base(&host, port)))
        .header("Authorization", format!("Bearer {secret}"))
        .header("X-Printer-Name", &printer)
        .header("Content-Type", "text/plain")
        .body(zpl)
        .send()
        .await;
    audit(&state, &p.email, "test_print", &id, json!({"printer": printer})).await;
    match resp {
        Ok(r) => Json(json!({ "ok": r.status().is_success(), "status": r.status().as_u16() })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

const NODE_SELECT: &str = "SELECT n.id, n.site, n.hostname, n.mgmt_port, n.app_version, \
    n.config_version, n.queue_depth, CAST(n.last_seen AS TEXT) AS last_seen, \
    (n.last_seen IS NOT NULL AND now() - n.last_seen < interval '90 seconds') AS online, \
    c.version AS desired_version, n.printers_json \
    FROM nodes n LEFT JOIN node_config c ON c.node_id = n.id";

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: String,
    site: String,
    hostname: String,
    mgmt_port: i32,
    app_version: String,
    config_version: i64,
    queue_depth: i32,
    last_seen: Option<String>,
    online: Option<bool>,
    desired_version: Option<i64>,
    printers_json: Value,
}

impl NodeRow {
    fn to_json(self) -> Value {
        json!({
            "id": self.id,
            "site": self.site,
            "hostname": self.hostname,
            "mgmtPort": self.mgmt_port,
            "appVersion": self.app_version,
            "reportedConfigVersion": self.config_version,
            "desiredConfigVersion": self.desired_version,
            "drift": self.desired_version.map(|d| d as i64 != self.config_version).unwrap_or(false),
            "queueDepth": self.queue_depth,
            "lastSeen": self.last_seen,
            "online": self.online.unwrap_or(false),
            "printers": self.printers_json,
        })
    }
}

async fn site_of(state: &AppState, id: &str) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT site FROM nodes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .map(|r| r.0)
}

async fn node_addr(state: &AppState, id: &str) -> Option<(String, i32)> {
    sqlx::query_as::<_, (String, i32)>("SELECT hostname, mgmt_port FROM nodes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .filter(|(h, _)| !h.is_empty())
}

async fn can_access(state: &AppState, p: &Principal, site: &str) -> bool {
    match allowed_sites(state, p).await {
        None => true,
        Some(sites) => sites.iter().any(|s| s == site),
    }
}

async fn push_refresh(state: &AppState, id: &str) -> bool {
    let Some((host, port)) = node_addr(state, id).await else {
        return false;
    };
    state
        .http
        .post(format!("{}/api/admin/refresh", state.node_base(&host, port)))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn audit(state: &AppState, actor: &str, action: &str, target: &str, detail: Value) {
    let _ = sqlx::query("INSERT INTO audit (actor, action, target, detail) VALUES ($1,$2,$3,$4)")
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(SqlxJson(detail))
        .execute(&state.db)
        .await;
}
