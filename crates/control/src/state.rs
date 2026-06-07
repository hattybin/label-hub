//! Shared control-plane state.

use std::sync::Arc;

use sqlx::postgres::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: PgPool,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(cfg: Config, db: PgPool) -> Self {
        AppState {
            cfg: Arc::new(cfg),
            db,
            http: reqwest::Client::builder()
                .user_agent("label-control")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("build reqwest client"),
        }
    }

    /// Base URL to reach a node's management API over the mesh.
    pub fn node_base(&self, hostname: &str, mgmt_port: i32) -> String {
        format!("{}://{}:{}", self.cfg.node_scheme, hostname, mgmt_port)
    }
}
