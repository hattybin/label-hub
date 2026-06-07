//! Job queue + history endpoints and the SSE stream that drives the console.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::routes::send_to_printer;
use crate::state::{now_iso, AppState, Job, JobStatus};

/// GET /api/jobs — pending (queued) jobs.
pub async fn list_pending(State(state): State<AppState>) -> impl IntoResponse {
    let store = state.store.lock().await;
    Json(store.pending.clone())
}

/// GET /api/jobs/history — recent printed/failed/dismissed jobs.
pub async fn list_history(State(state): State<AppState>) -> impl IntoResponse {
    let store = state.store.lock().await;
    Json(store.history.clone())
}

#[derive(Deserialize)]
pub struct PrintQuery {
    /// Optional printer override (used for reprints to a different printer).
    pub printer: Option<String>,
}

/// POST /api/jobs/:id/print — release a queued job, or reprint a historical one.
pub async fn print_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PrintQuery>,
) -> impl IntoResponse {
    // Find the job in pending first, otherwise in history (reprint).
    let (mut job, was_pending) = {
        let store = state.store.lock().await;
        if let Some(j) = store.pending.iter().find(|j| j.id == id).cloned() {
            (j, true)
        } else if let Some(j) = store.history.iter().find(|j| j.id == id).cloned() {
            (j, false)
        } else {
            return (StatusCode::NOT_FOUND, "job not found").into_response();
        }
    };

    let target_printer = q
        .printer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| job.printer.clone());

    let result = send_to_printer(&state, &target_printer, &job.zpl).await;

    match result {
        Ok(()) => {
            if was_pending {
                // Move from pending → history as printed.
                let mut store = state.store.lock().await;
                store.pending.retain(|j| j.id != id);
                job.status = JobStatus::Printed;
                job.printer = target_printer.clone();
                job.printed_at = Some(now_iso());
                job.error = None;
                store.history.insert(0, job.clone());
            } else {
                // Reprint: create a fresh history entry.
                let mut store = state.store.lock().await;
                job = Job {
                    id: Uuid::new_v4().to_string(),
                    printer: target_printer.clone(),
                    zpl: job.zpl.clone(),
                    source: "reprint".to_string(),
                    received_at: now_iso(),
                    status: JobStatus::Printed,
                    printed_at: Some(now_iso()),
                    error: None,
                    label_count: job.label_count,
                };
                store.history.insert(0, job.clone());
            }
            state.save_jobs().await;
            crate::agent::report_event(
                &state,
                label_proto::PrintEvent {
                    printer: job.printer.clone(),
                    status: "printed".to_string(),
                    source: job.source.clone(),
                    at: now_iso(),
                    error: None,
                },
            );
            state.broadcast(&json!({ "type": "job_update", "job": job }));
            (StatusCode::OK, Json(json!({ "ok": true, "id": job.id }))).into_response()
        }
        Err(e) => {
            // Record the failure but keep a queued job in the queue for retry.
            if was_pending {
                let mut store = state.store.lock().await;
                if let Some(j) = store.pending.iter_mut().find(|j| j.id == id) {
                    j.status = JobStatus::Failed;
                    j.error = Some(e.clone());
                }
            }
            state.save_jobs().await;
            state.broadcast(&json!({ "type": "job_error", "id": id, "error": e }));
            (StatusCode::BAD_GATEWAY, Json(json!({ "ok": false, "error": e }))).into_response()
        }
    }
}

/// POST /api/jobs/:id/dismiss — remove a queued job without printing.
pub async fn dismiss_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let removed = {
        let mut store = state.store.lock().await;
        if let Some(pos) = store.pending.iter().position(|j| j.id == id) {
            let mut j = store.pending.remove(pos);
            j.status = JobStatus::Dismissed;
            store.history.insert(0, j);
            true
        } else {
            false
        }
    };
    if !removed {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    }
    state.save_jobs().await;
    state.broadcast(&json!({ "type": "job_dismissed", "id": id }));
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// GET /api/queue-events — SSE stream. Sends the current pending backlog on
/// connect, then live events as jobs arrive / change.
pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let backlog = {
        let store = state.store.lock().await;
        json!({ "type": "backlog", "jobs": store.pending.clone() }).to_string()
    };
    let rx = state.tx.subscribe();

    let initial = stream::once(async move { Ok(Event::default().data(backlog)) });
    let live = BroadcastStream::new(rx).filter_map(|msg| async move {
        match msg {
            Ok(data) => Some(Ok(Event::default().data(data))),
            Err(_) => None, // lagged receiver — skip
        }
    });

    Sse::new(initial.chain(live)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("hb"),
    )
}
