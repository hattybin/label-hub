//! Mint per-node Tailscale auth keys via a Tailscale OAuth client.
//!
//! Tailscale auth keys created from an OAuth client must be tagged; we issue a
//! pre-authorized, non-reusable, tagged key per node so the device can join
//! unattended (`tailscale up --auth-key=...`) and inherit tag-based ACLs.

use serde_json::json;

use crate::state::AppState;

/// Returns a fresh tagged auth key, or None if Tailscale isn't configured / fails.
pub async fn mint_authkey(state: &AppState) -> Option<String> {
    let cfg = &state.cfg;
    if !cfg.tailscale_enabled() {
        return None;
    }
    let client_id = cfg.ts_oauth_client_id.as_ref()?;
    let client_secret = cfg.ts_oauth_client_secret.as_ref()?;
    let tailnet = cfg.ts_tailnet.as_ref()?;

    // 1. OAuth client-credentials token.
    let token_resp = state
        .http
        .post("https://api.tailscale.com/api/v2/oauth/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
        .ok()?;
    if !token_resp.status().is_success() {
        tracing::warn!("tailscale oauth token failed: {}", token_resp.status());
        return None;
    }
    let tok: serde_json::Value = token_resp.json().await.ok()?;
    let access = tok.get("access_token")?.as_str()?.to_string();

    // 2. Create a tagged, pre-authorized, single-use auth key.
    let body = json!({
        "capabilities": { "devices": { "create": {
            "reusable": false,
            "ephemeral": false,
            "preauthorized": true,
            "tags": [cfg.ts_tag]
        }}},
        "expirySeconds": 86400
    });
    let key_resp = state
        .http
        .post(format!("https://api.tailscale.com/api/v2/tailnet/{tailnet}/keys"))
        .bearer_auth(access)
        .json(&body)
        .send()
        .await
        .ok()?;
    if !key_resp.status().is_success() {
        tracing::warn!("tailscale key create failed: {}", key_resp.status());
        return None;
    }
    let key: serde_json::Value = key_resp.json().await.ok()?;
    key.get("key").and_then(|k| k.as_str()).map(|s| s.to_string())
}
