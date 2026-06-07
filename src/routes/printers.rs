//! Printer profile CRUD and a TCP reachability probe. Profiles are intentionally
//! minimal: a name, an IP/host, and a port — "simple basic address and names only".

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::state::{AppState, Printer};

/// GET /api/printers
pub async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let store = state.store.lock().await;
    Json(store.printers.clone())
}

#[derive(Deserialize)]
pub struct PrinterInput {
    pub name: String,
    pub ip: String,
    pub port: Option<u16>,
}

/// POST /api/printers — create or update (upsert by name).
pub async fn upsert(
    State(state): State<AppState>,
    Json(input): Json<PrinterInput>,
) -> impl IntoResponse {
    let name = input.name.trim().to_string();
    let ip = input.ip.trim().to_string();
    if name.is_empty() || ip.is_empty() {
        return (StatusCode::BAD_REQUEST, "name and ip are required").into_response();
    }
    let entry = Printer {
        name: name.clone(),
        ip,
        port: input.port.unwrap_or(9100),
    };
    {
        let mut store = state.store.lock().await;
        if let Some(p) = store
            .printers
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(&name))
        {
            *p = entry.clone();
        } else {
            store.printers.push(entry.clone());
        }
    }
    state.save_printers().await;
    (StatusCode::OK, Json(entry)).into_response()
}

/// DELETE /api/printers/:name
pub async fn remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    {
        let mut store = state.store.lock().await;
        store.printers.retain(|p| !p.name.eq_ignore_ascii_case(&name));
    }
    state.save_printers().await;
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct TestQuery {
    pub ip: String,
    pub port: Option<u16>,
}

/// GET /api/test-printer?ip=&port=
pub async fn test(Query(q): Query<TestQuery>) -> impl IntoResponse {
    let port = q.port.unwrap_or(9100);
    let reachable = crate::printer::is_reachable(&q.ip, port).await;
    Json(json!({ "reachable": reachable, "ip": q.ip, "port": port }))
}
