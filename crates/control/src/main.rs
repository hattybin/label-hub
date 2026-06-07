//! label-control — central control plane (C2) for the label-hub fleet.
//!
//! Two listeners:
//!   * NODE API (tailnet-only in prod): enroll / heartbeat / config / events.
//!   * DASHBOARD (Entra SSO via EasyAuth in prod): fleet view, config, actions.

mod auth;
mod config;
mod routes;
mod state;
mod tailscale;
mod util;

use std::net::SocketAddr;

use axum::{
    routing::{get, post, put},
    Router,
};
use sqlx::postgres::PgPoolOptions;
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
                .unwrap_or_else(|_| "label_control=info,tower_http=warn,sqlx=warn".into()),
        )
        .init();

    let cfg = Config::from_env();

    // Retry the DB connection — Postgres (esp. a fresh container) may not be ready
    // the instant we start.
    let db = {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match PgPoolOptions::new()
                .max_connections(10)
                .connect(&cfg.database_url)
                .await
            {
                Ok(pool) => break pool,
                Err(e) if attempt < 15 => {
                    tracing::warn!("Postgres not ready (attempt {attempt}): {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => panic!("connect to Postgres (DATABASE_URL): {e}"),
            }
        }
    };

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("run migrations");

    let web_dir = std::env::var("DASH_WEB_DIR").unwrap_or_else(|_| "web".into());
    let state = AppState::new(cfg, db);

    // ── Node API (tailnet-only in prod) ───────────────────────────────────────
    let node_api = Router::new()
        .route("/api/enroll", post(routes::node_api::enroll))
        .route("/api/nodes/:id/heartbeat", post(routes::node_api::heartbeat))
        .route("/api/nodes/:id/config", get(routes::node_api::get_config))
        .route("/api/nodes/:id/events", post(routes::node_api::events))
        .route("/healthz", get(|| async { "ok" }))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    // ── Dashboard (Entra SSO via EasyAuth in prod) ────────────────────────────
    let serve_dir = ServeDir::new(&web_dir).not_found_service(ServeFile::new(format!("{web_dir}/index.html")));
    let dash = Router::new()
        .route("/dash/me", get(routes::dash::me))
        .route("/dash/nodes", get(routes::dash::list_nodes))
        .route("/dash/nodes/:id", get(routes::dash::get_node))
        .route("/dash/nodes/:id/config", put(routes::dash::update_config))
        .route("/dash/nodes/:id/events", get(routes::dash::node_events))
        .route("/dash/nodes/:id/test-print", post(routes::dash::test_print))
        .route(
            "/dash/enrollment-tokens",
            get(routes::dash::list_tokens).post(routes::dash::create_token),
        )
        .fallback_service(serve_dir)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let node_addr: SocketAddr = format!("{}:{}", state.cfg.node_api_bind, state.cfg.node_api_port)
        .parse()
        .expect("node api addr");
    let dash_addr: SocketAddr = format!("{}:{}", state.cfg.dash_bind, state.cfg.dash_port)
        .parse()
        .expect("dash addr");

    let node_listener = tokio::net::TcpListener::bind(node_addr).await.expect("bind node api");
    let dash_listener = tokio::net::TcpListener::bind(dash_addr).await.expect("bind dash");

    tracing::info!("label-control up");
    tracing::info!("  node API : http://{node_addr}  (tailnet-only in prod)");
    tracing::info!("  dashboard: http://{dash_addr}");
    tracing::info!("  tailscale: {}", if state.cfg.tailscale_enabled() { "enabled" } else { "disabled" });
    if state.cfg.dev_admin.is_some() {
        tracing::warn!("  DEV_ADMIN set — dashboard auth bypass active (do not use in prod)");
    }

    let n = axum::serve(node_listener, node_api).with_graceful_shutdown(shutdown_signal());
    let d = axum::serve(dash_listener, dash).with_graceful_shutdown(shutdown_signal());
    let (_a, _b) = tokio::join!(n, d);
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("ctrl-c") };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
