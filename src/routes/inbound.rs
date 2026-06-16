//! The D365 external-service webhook. D365's "Print labels using an external
//! service" feature POSTs here. The request format is whatever we define in the
//! D365 External Service *operation* — we accept two shapes:
//!
//!   1. Raw ZPL body  + `X-Printer-Name` header  (recommended, no escaping)
//!         Authorization: Bearer <INBOUND_SECRET>   (D365 $auth.secret$)
//!         X-Printer-Name: <printer>                (D365 $label.printer$)
//!         Content-Type: text/plain
//!         <raw ZPL>                                (D365 $label.body$)
//!
//!   2. JSON body: { "printer": "...", "zplBase64": "..." }
//!         using D365 $label.body:base64$
//!
//! D365 treats any non-2xx response as a failure (and logs it), so we return 200
//! once a job is accepted (queued or printed) and 4xx for auth/config errors.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::routes::send_to_printer;
use crate::state::{count_labels, now_iso, AppState, Job, JobStatus};

#[derive(Deserialize)]
struct JsonBody {
    printer: Option<String>,
    #[serde(default)]
    zpl: Option<String>,
    #[serde(default)]
    zpl_base64: Option<String>,
    #[serde(rename = "zplBase64", default)]
    zpl_base64_camel: Option<String>,
}

pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Authenticate against the shared secret (runtime value; the control plane
    //    can rotate it without a restart).
    let expected_secret = state.inbound_secret().await;
    if !secret_ok(&expected_secret, &headers) {
        tracing::warn!("inbound: rejected request with invalid/missing secret");
        return (StatusCode::UNAUTHORIZED, "invalid secret").into_response();
    }

    // 2. Site filter — if D365_SITE_FILTER is set, reject jobs whose X-Site header
    //    doesn't match. Prevents misdirected jobs when multiple nodes share a D365
    //    external service config.  Leave blank to accept jobs from any site.
    if let Some(filter) = &state.config.site_filter {
        let site = headers
            .get("x-site")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim())
            .unwrap_or("");
        if !site.eq_ignore_ascii_case(filter) {
            tracing::warn!("inbound: site filter rejected '{}' (expected '{}')", site, filter);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("site filter: expected '{filter}', got '{site}'"),
            )
                .into_response();
        }
    }

    // 3. Determine printer name + ZPL from whichever body shape was used.
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let (printer_name, zpl) = if content_type.contains("application/json") {
        match parse_json(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    } else {
        let printer = headers
            .get("x-printer-name")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| state.config.default_printer.clone());
        let zpl = String::from_utf8_lossy(&body).to_string();
        match printer {
            Some(p) => (p, zpl),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    "missing printer (X-Printer-Name header or DEFAULT_PRINTER)",
                )
                    .into_response()
            }
        }
    };

    if zpl.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty label body").into_response();
    }

    // 4. Build the job.
    let mut job = Job {
        id: Uuid::new_v4().to_string(),
        printer: printer_name.clone(),
        label_count: count_labels(&zpl),
        zpl,
        source: "d365".to_string(),
        received_at: now_iso(),
        status: JobStatus::Queued,
        printed_at: None,
        error: None,
    };

    // 5. Verify the printer exists before deciding what to do.
    let known = {
        let store = state.store.lock().await;
        crate::routes::find_printer(&store.printers, &printer_name).is_some()
    };
    if !known {
        job.status = JobStatus::Failed;
        job.error = Some(format!("unknown printer '{printer_name}'"));
        {
            let mut store = state.store.lock().await;
            store.history.insert(0, job.clone());
        }
        state.save_jobs().await;
        state.broadcast(&json!({ "type": "job_update", "job": job }));
        tracing::warn!("inbound: unknown printer '{}'", printer_name);
        return (StatusCode::UNPROCESSABLE_ENTITY, format!("unknown printer '{printer_name}'"))
            .into_response();
    }

    let auto_print = { state.store.lock().await.settings.auto_print };

    if auto_print {
        // Print immediately, record in history.
        match send_to_printer(&state, &printer_name, &job.zpl).await {
            Ok(()) => {
                job.status = JobStatus::Printed;
                job.printed_at = Some(now_iso());
                tracing::info!("inbound: auto-printed job {} to '{}'", job.id, printer_name);
            }
            Err(e) => {
                job.status = JobStatus::Failed;
                job.error = Some(e.clone());
                tracing::error!("inbound: auto-print failed for '{}': {}", printer_name, e);
            }
        }
        {
            let mut store = state.store.lock().await;
            store.history.insert(0, job.clone());
        }
        state.save_jobs().await;
        crate::agent::report_event(
            &state,
            label_proto::PrintEvent {
                printer: job.printer.clone(),
                status: format!("{:?}", job.status).to_lowercase(),
                source: job.source.clone(),
                at: now_iso(),
                error: job.error.clone(),
            },
        );
        state.broadcast(&json!({ "type": "job_update", "job": job }));
    } else {
        // Hold in the queue for an operator to release.
        {
            let mut store = state.store.lock().await;
            store.pending.insert(0, job.clone());
        }
        state.save_jobs().await;
        state.broadcast(&json!({ "type": "new_job", "job": job }));
        tracing::info!("inbound: queued job {} for '{}'", job.id, printer_name);
    }

    // D365 considers the label handled once we return 200.
    (StatusCode::OK, Json(json!({ "ok": true, "id": job.id, "status": job.status }))).into_response()
}

fn secret_ok(expected_secret: &str, headers: &HeaderMap) -> bool {
    let expected = expected_secret.as_bytes();
    if expected.is_empty() {
        return false; // no secret configured → reject everything
    }
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v).trim())
        // Allow a plain X-Auth-Secret header as an alternative to Authorization.
        .or_else(|| headers.get("x-auth-secret").and_then(|v| v.to_str().ok()))
        .unwrap_or("");
    constant_time_eq(presented.as_bytes(), expected)
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

fn parse_json(body: &Bytes) -> Result<(String, String), String> {
    let parsed: JsonBody =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))?;
    let printer = parsed
        .printer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "JSON body missing 'printer'".to_string())?;

    let zpl = if let Some(raw) = parsed.zpl.filter(|s| !s.is_empty()) {
        raw
    } else if let Some(b64) = parsed
        .zpl_base64
        .or(parsed.zpl_base64_camel)
        .filter(|s| !s.is_empty())
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("invalid base64 in zplBase64: {e}"))?;
        String::from_utf8(bytes).map_err(|e| format!("zplBase64 is not valid UTF-8: {e}"))?
    } else {
        return Err("JSON body missing 'zpl' or 'zplBase64'".to_string());
    };

    Ok((printer, zpl))
}
