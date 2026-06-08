//! label-hub — a self-hosted ZPL label print hub for Dynamics 365 F&O / SCM.
//!
//! Two HTTP listeners run in one process:
//!   * PUBLIC  — exposes only the D365 webhook (`/api/print/inbound`). Bound to
//!               loopback by default; a tunnel sidecar (cloudflared / azbridge) on
//!               this host forwards public HTTPS to it. Secret-protected.
//!   * LOCAL   — the web console + all management/settings APIs. Bound to the LAN
//!               and (optionally) advertised over mDNS as `printlabels.local`.
//!
//! Splitting the two means the public tunnel can never reach the console or
//! printer/settings APIs — only the print webhook — while the LAN side stays
//! relaxed and easy to reach by name.

mod agent;
mod config;
mod d365_client;
mod mdns;
mod printer;
mod routes;
mod state;

use std::net::SocketAddr;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "label_hub=info,tower_http=warn".into()),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::new(config);

    // ── Public listener: webhook only ────────────────────────────────────────
    let public_app = Router::new()
        .route("/api/print/inbound", post(routes::inbound::handle))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024)) // large batch ZPL
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    // ── Local listener: console + management ─────────────────────────────────
    let serve_dir = ServeDir::new("web").not_found_service(ServeFile::new("web/index.html"));
    let local_app = Router::new()
        // The webhook is also reachable locally (handy for testing on the LAN).
        .route("/api/print/inbound", post(routes::inbound::handle))
        .route("/api/queue-events", get(routes::jobs::events))
        .route("/api/jobs", get(routes::jobs::list_pending))
        .route("/api/jobs/history", get(routes::jobs::list_history))
        .route("/api/jobs/:id/print", post(routes::jobs::print_job))
        .route("/api/jobs/:id/dismiss", post(routes::jobs::dismiss_job))
        .route("/api/printers", get(routes::printers::list).post(routes::printers::upsert))
        .route("/api/printers/:name", delete(routes::printers::remove))
        .route("/api/test-printer", get(routes::printers::test))
        .route("/api/preview-label", post(routes::preview::preview))
        .route("/api/settings", get(routes::settings::get).put(routes::settings::put))
        .route("/api/admin/refresh", post(routes::settings::refresh))
        .route("/api/admin/update", post(routes::settings::update))
        .route("/api/health", get(routes::settings::health))
        .route("/api/d365/health", get(routes::d365::health))
        .route("/api/d365/query", get(routes::d365::query))
        // PO
        .route("/api/d365/po/:po_number", get(routes::d365::get_po))
        .route("/api/d365/pos-by-vendor/:vendor_account", get(routes::d365::get_pos_by_vendor))
        // Receipts
        .route("/api/d365/receipts-for-po/:po_number", get(routes::d365::get_receipts_for_po))
        .route("/api/d365/receipt/:receipt_number", get(routes::d365::get_receipt))
        .route("/api/d365/recent-receipts", get(routes::d365::recent_receipts))
        // Products
        .route("/api/d365/product-descriptions", get(routes::d365::product_descriptions))
        .route("/api/d365/product/:item_number", get(routes::d365::get_product))
        // Discovery
        .route("/api/d365/discover-entities", get(routes::d365::discover_entities))
        // Inspect / field-discovery
        .route("/api/d365/inspect/receipt", get(routes::d365::inspect_receipt))
        .route("/api/d365/inspect/receipt-line", get(routes::d365::inspect_receipt_line))
        .route("/api/d365/inspect/po-line", get(routes::d365::inspect_po_line))
        .route("/api/d365/inspect/product", get(routes::d365::inspect_product))
        .fallback_service(serve_dir)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let cfg = &state.config;
    let public_addr: SocketAddr = format!("{}:{}", cfg.public_bind, cfg.public_port)
        .parse()
        .unwrap_or_else(|e| panic!("invalid public bind address: {e}"));
    let local_addr: SocketAddr = format!("{}:{}", cfg.local_bind, cfg.local_port)
        .parse()
        .unwrap_or_else(|e| panic!("invalid local bind address: {e}"));

    let public_listener = tokio::net::TcpListener::bind(public_addr)
        .await
        .unwrap_or_else(|e| panic!("could not bind public {public_addr}: {e}"));
    let local_listener = tokio::net::TcpListener::bind(local_addr)
        .await
        .unwrap_or_else(|e| panic!("could not bind local {local_addr}: {e}"));

    // ── Optional mDNS advertisement for the local console ─────────────────────
    let _mdns = if cfg.mdns_enable {
        match mdns::advertise(&cfg.mdns_fqdn(), cfg.local_port, &cfg.site_name) {
            Ok(daemon) => {
                tracing::info!(
                    "mDNS: console advertised at http://{}:{}",
                    cfg.mdns_fqdn(),
                    cfg.local_port
                );
                Some(daemon)
            }
            Err(e) => {
                tracing::warn!("mDNS advertisement disabled: {e}");
                None
            }
        }
    } else {
        None
    };

    banner(&state);

    // Control-plane agent (no-op in standalone mode).
    tokio::spawn(agent::run(state.clone()));

    let public = axum::serve(public_listener, public_app).with_graceful_shutdown(shutdown_signal());
    let local = axum::serve(local_listener, local_app).with_graceful_shutdown(shutdown_signal());

    let (rp, rl) = tokio::join!(public, local);
    if let Err(e) = rp {
        tracing::error!("public listener error: {e}");
    }
    if let Err(e) = rl {
        tracing::error!("local listener error: {e}");
    }
}

fn banner(state: &AppState) {
    let cfg = &state.config;
    tracing::info!("label-hub starting — site: {}", cfg.site_name);
    tracing::info!(
        "  PUBLIC (D365 webhook): http://{}:{}/api/print/inbound",
        cfg.public_bind,
        cfg.public_port
    );
    tracing::info!(
        "  LOCAL  (console)     : http://{}:{}{}",
        cfg.local_bind,
        cfg.local_port,
        if cfg.mdns_enable {
            format!("   (also http://{}:{})", cfg.mdns_fqdn(), cfg.local_port)
        } else {
            String::new()
        }
    );
    tracing::info!(
        "  inbound secret       : {}",
        if cfg.inbound_secret.is_empty() { "⚠ NOT SET (webhook will reject)" } else { "✓ set" }
    );
    tracing::info!(
        "  public URL (for D365): {}",
        cfg.public_url.as_deref().unwrap_or("(set PUBLIC_URL to your tunnel host)")
    );
    tracing::info!(
        "  auto-print default   : {}   default printer: {}",
        cfg.auto_print_default,
        cfg.default_printer.as_deref().unwrap_or("(none)")
    );
    tracing::info!(
        "  D365 OData           : {}",
        if cfg.d365_enabled() { "✓ enabled" } else { "disabled (optional)" }
    );
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
