//! Receiving label print workflow — builds ZPL from a template, sends to a
//! named Zebra printer, and optionally renders a preview via Labelary.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

// ── Data model ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivingLabel {
    #[serde(default)] pub item_number: String,
    #[serde(default)] pub item_desc: String,
    #[serde(default)] pub po_number: String,
    #[serde(default)] pub search_name: String,
    #[serde(default)] pub product_name: String,
    #[serde(default)] pub unit: String,
    #[serde(default)] pub config_id: String,
    #[serde(default)] pub size_id: String,
    #[serde(default)] pub color_id: String,
    #[serde(default)] pub batch_id: String,
    #[serde(default)] pub serial_id: String,
    #[serde(default)] pub warehouse_id: String,
    #[serde(default)] pub bin_location: String,
    #[serde(default = "one")] pub count: u32,
}

fn one() -> u32 { 1 }

// ── ZPL builder ──────────────────────────────────────────────────────────────

// Embedded at compile time; lives in web/ so the update script syncs it to the
// Pi alongside HTML/JS.  Edit web/receiving-label.zpl and rebuild to change it.
const ZPL_TEMPLATE: &str = include_str!("../../web/receiving-label.zpl");

fn build_zpl(label: &ReceivingLabel) -> String {
    let s = |v: &str| crate::printer::sanitize_field(v);
    ZPL_TEMPLATE
        .replace("{{item}}", &s(&label.item_number))
        .replace("{{name}}", &s(&label.item_desc))
        .replace("{{po}}", &s(&label.po_number))
        .replace("{{search}}", &s(&label.search_name))
        .replace("{{productname}}", &s(&label.product_name))
        .replace("{{unit}}", &s(&label.unit))
        .replace("{{config}}", &s(&label.config_id))
        .replace("{{size}}", &s(&label.size_id))
        .replace("{{color}}", &s(&label.color_id))
        .replace("{{batch}}", &s(&label.batch_id))
        .replace("{{serial}}", &s(&label.serial_id))
        .replace("{{wh}}", &s(&label.warehouse_id))
        .replace("{{bin}}", &s(&label.bin_location))
}

// ── Print ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintRequest {
    pub printer: String,
    pub labels: Vec<ReceivingLabel>,
}

/// POST /api/receiving-labels/print
pub async fn print(
    State(state): State<AppState>,
    Json(req): Json<PrintRequest>,
) -> impl IntoResponse {
    if req.printer.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "printer is required" }))).into_response();
    }
    if req.labels.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "labels array is empty" }))).into_response();
    }

    let mut parts: Vec<String> = Vec::new();
    for label in &req.labels {
        let count = label.count.clamp(0, 999);
        if count == 0 { continue; }
        let zpl = build_zpl(label);
        for _ in 0..count {
            parts.push(zpl.clone());
        }
    }

    if parts.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "all labels have count 0" }))).into_response();
    }

    let total = parts.len();
    let combined = parts.join("\n");

    match super::send_to_printer(&state, req.printer.trim(), &combined).await {
        Ok(()) => Json(json!({ "ok": true, "count": total })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

// ── Preview ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PreviewRequest {
    pub label: ReceivingLabel,
    pub size: Option<String>,
}

/// POST /api/receiving-labels/preview — build ZPL and render via Labelary.
pub async fn preview(
    State(state): State<AppState>,
    Json(req): Json<PreviewRequest>,
) -> impl IntoResponse {
    let zpl = build_zpl(&req.label);
    let size = req.size.unwrap_or_else(|| "4x2".to_string());
    let url = format!("https://api.labelary.com/v1/printers/8dpmm/labels/{size}/0/");

    match state.http
        .post(&url)
        .header("Accept", "image/png")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(zpl)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(bytes) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                Json(json!({ "image": format!("data:image/png;base64,{b64}") })).into_response()
            }
            Err(e) => (StatusCode::BAD_GATEWAY, format!("read failed: {e}")).into_response(),
        },
        Ok(r) => (StatusCode::BAD_GATEWAY, format!("Labelary HTTP {}", r.status().as_u16())).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Labelary unreachable: {e}")).into_response(),
    }
}
