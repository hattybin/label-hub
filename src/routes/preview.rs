//! ZPL → PNG preview via the public Labelary web service, mirroring the Node
//! prototype's `/api/preview-label`. The console uses this to show operators what
//! a queued/historical label looks like before printing.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct PreviewInput {
    pub zpl: String,
    /// Label size in inches, e.g. "4x2" (defaults to 4x2 at 8 dots/mm = 203 DPI).
    #[serde(default)]
    pub size: Option<String>,
}

pub async fn preview(
    State(state): State<AppState>,
    Json(input): Json<PreviewInput>,
) -> impl IntoResponse {
    if input.zpl.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "zpl is required").into_response();
    }
    let size = input.size.unwrap_or_else(|| "4x2".to_string());
    let url = format!("https://api.labelary.com/v1/printers/8dpmm/labels/{size}/0/");

    let resp = state
        .http
        .post(&url)
        .header("Accept", "image/png")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(input.zpl)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(bytes) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                Json(json!({ "image": format!("data:image/png;base64,{b64}") })).into_response()
            }
            Err(e) => (StatusCode::BAD_GATEWAY, format!("read preview failed: {e}")).into_response(),
        },
        Ok(r) => (
            StatusCode::BAD_GATEWAY,
            format!("Labelary returned HTTP {}", r.status().as_u16()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("could not reach Labelary: {e}"),
        )
            .into_response(),
    }
}
