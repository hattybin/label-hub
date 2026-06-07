//! Optional D365 OData lookup routes. Enabled only when AZURE_*/D365_BASE_URL are
//! configured. Useful for ad-hoc browse/lookup; not required for label printing.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::d365_client;
use crate::state::AppState;

/// GET /api/d365/health — verifies token acquisition.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.config.d365_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "enabled": false, "reason": "AZURE_*/D365_BASE_URL not configured" })),
        )
            .into_response();
    }
    match d365_client::get_token(&state).await {
        Ok(_) => Json(json!({ "enabled": true, "token": "ok" })).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "enabled": true, "token": "error", "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct QueryParams {
    pub entity: String,
    pub filter: Option<String>,
    pub select: Option<String>,
    pub top: Option<u32>,
    #[serde(rename = "crossCompany")]
    pub cross_company: Option<bool>,
}

/// GET /api/d365/query?entity=&filter=&select=&top= — thin OData passthrough.
pub async fn query(
    State(state): State<AppState>,
    Query(p): Query<QueryParams>,
) -> impl IntoResponse {
    if !state.config.d365_enabled() {
        return (StatusCode::SERVICE_UNAVAILABLE, "D365 OData not configured").into_response();
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = p.filter.filter(|s| !s.is_empty()) {
        parts.push(format!("$filter={}", urlencoding(&f)));
    }
    if let Some(s) = p.select.filter(|s| !s.is_empty()) {
        parts.push(format!("$select={}", urlencoding(&s)));
    }
    parts.push(format!("$top={}", p.top.unwrap_or(50)));
    if p.cross_company.unwrap_or(true) {
        parts.push("cross-company=true".to_string());
    }
    let qs = parts.join("&");

    match d365_client::odata_get(&state, &p.entity, &qs).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// Minimal percent-encoding for OData query values (space, quotes, etc.).
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
