//! D365 OData lookup routes. Enabled only when AZURE_*/D365_BASE_URL are set.
//! These power the optional manual-lookup feature; the core print path does not
//! depend on them.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::d365_client;
use crate::state::AppState;

// ── Guard macro ──────────────────────────────────────────────────────────────

macro_rules! d365_check {
    ($state:expr) => {
        if !$state.config.d365_enabled() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "D365 OData not configured (AZURE_*/D365_BASE_URL missing)" })),
            )
                .into_response();
        }
    };
}

// ── Health / token probe ─────────────────────────────────────────────────────

/// GET /api/d365/health — verifies token acquisition.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.config.d365_enabled() {
        return Json(json!({ "enabled": false, "reason": "AZURE_*/D365_BASE_URL not configured" }))
            .into_response();
    }
    match d365_client::get_token(&state).await {
        Ok(_) => Json(json!({ "enabled": true, "token": "ok" })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "enabled": true, "error": e }))).into_response(),
    }
}

// ── Generic passthrough ──────────────────────────────────────────────────────

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
pub async fn query(State(state): State<AppState>, Query(p): Query<QueryParams>) -> impl IntoResponse {
    d365_check!(state);
    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = p.filter.filter(|s| !s.is_empty()) {
        parts.push(format!("$filter={}", urlenc(&f)));
    }
    if let Some(s) = p.select.filter(|s| !s.is_empty()) {
        parts.push(format!("$select={}", urlenc(&s)));
    }
    parts.push(format!("$top={}", p.top.unwrap_or(50)));
    if p.cross_company.unwrap_or(true) {
        parts.push("cross-company=true".to_string());
    }
    match d365_client::odata_get(&state, &p.entity, &parts.join("&")).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── PO routes ────────────────────────────────────────────────────────────────

/// GET /api/d365/po/:po_number — PO header + lines.
pub async fn get_po(State(state): State<AppState>, Path(po_number): Path<String>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::get_po_with_lines(&state, &po_number).await {
        Ok(Some(po)) => Json(json!(po)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("PO '{}' not found", po_number) }))).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /api/d365/pos-by-vendor/:vendor_account — POs matching a vendor account.
pub async fn get_pos_by_vendor(State(state): State<AppState>, Path(vendor): Path<String>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::get_pos_by_vendor(&state, &vendor).await {
        Ok(pos) => Json(json!(pos)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── Receipt routes ───────────────────────────────────────────────────────────

/// GET /api/d365/receipts-for-po/:po_number — all receipt headers for a PO.
pub async fn get_receipts_for_po(State(state): State<AppState>, Path(po_number): Path<String>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::get_receipts_for_po(&state, &po_number).await {
        Ok(receipts) => Json(json!(receipts)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /api/d365/receipt/:receipt_number — receipt header + lines.
pub async fn get_receipt(State(state): State<AppState>, Path(receipt_number): Path<String>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::get_receipt_with_lines(&state, &receipt_number).await {
        Ok(Some(r)) => Json(json!(r)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("receipt '{}' not found", receipt_number) }))).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct RecentParams {
    #[serde(rename = "daysBack", default)]
    pub days_back: i64,
    #[serde(rename = "daysForward", default)]
    pub days_forward: i64,
}

/// GET /api/d365/recent-receipts?daysBack=0&daysForward=0 — receipts in a date window.
pub async fn recent_receipts(State(state): State<AppState>, Query(p): Query<RecentParams>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::get_recent_receipts(&state, p.days_back, p.days_forward).await {
        Ok(receipts) => Json(json!(receipts)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── Product routes ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ItemsParams {
    pub items: String,
}

/// GET /api/d365/product-descriptions?items=0000907,0000908 — batch product lookup.
pub async fn product_descriptions(State(state): State<AppState>, Query(p): Query<ItemsParams>) -> impl IntoResponse {
    d365_check!(state);
    let item_numbers: Vec<String> = p.items.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if item_numbers.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "items param required (comma-separated)" }))).into_response();
    }
    match d365_client::get_product_descriptions(&state, &item_numbers).await {
        Ok(map) => Json(json!(map)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /api/d365/product/:item_number — single product lookup.
pub async fn get_product(State(state): State<AppState>, Path(item_number): Path<String>) -> impl IntoResponse {
    d365_check!(state);
    let items = vec![item_number.clone()];
    match d365_client::get_product_descriptions(&state, &items).await {
        Ok(mut map) => match map.remove(&item_number) {
            Some(p) => Json(json!({ "itemNumber": item_number, "desc": p.desc, "searchName": p.search_name, "productName": p.product_name })).into_response(),
            None => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("item '{}' not found", item_number) }))).into_response(),
        },
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── Entity discovery ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DiscoverParams {
    #[serde(default = "default_pattern")]
    pub pattern: String,
}
fn default_pattern() -> String { "receipt".to_string() }

/// GET /api/d365/discover-entities?pattern=receipt — search $metadata entity names.
pub async fn discover_entities(State(state): State<AppState>, Query(p): Query<DiscoverParams>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::discover_entities(&state, &p.pattern).await {
        Ok(entities) => Json(json!({ "pattern": p.pattern, "count": entities.len(), "entities": entities })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── Inspect / field-discovery ────────────────────────────────────────────────

/// GET /api/d365/inspect/receipt — all fields from one receipt header record.
pub async fn inspect_receipt(State(state): State<AppState>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::inspect_receipt_header(&state).await {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ReceiptNumberParam { #[serde(rename = "receiptNumber")] pub receipt_number: String }
#[derive(Deserialize)]
pub struct ItemNumberParam { #[serde(rename = "itemNumber")] pub item_number: String }

/// GET /api/d365/inspect/receipt-line?receiptNumber=... — all fields on one line.
pub async fn inspect_receipt_line(State(state): State<AppState>, Query(p): Query<ReceiptNumberParam>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::inspect_receipt_line(&state, &p.receipt_number).await {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /api/d365/inspect/po-line?itemNumber=... — all fields on one PO line.
pub async fn inspect_po_line(State(state): State<AppState>, Query(p): Query<ItemNumberParam>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::inspect_po_line(&state, &p.item_number).await {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /api/d365/inspect/product?itemNumber=... — all fields on a released product.
pub async fn inspect_product(State(state): State<AppState>, Query(p): Query<ItemNumberParam>) -> impl IntoResponse {
    d365_check!(state);
    match d365_client::inspect_product(&state, &p.item_number).await {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

// ── URL encoding helper (values only; keys like $filter are written verbatim) ──

fn urlenc(s: &str) -> String {
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
