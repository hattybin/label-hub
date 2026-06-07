//! Optional D365 F&O OData client using Entra ID app (client-credential) auth.
//! Ported from the Node prototype's `d365Client.js`. This powers the optional
//! manual-lookup feature only; the core inbound print path does not need it.

use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::state::AppState;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

/// Acquire (and cache) an OData bearer token via the client-credentials flow.
pub async fn get_token(state: &AppState) -> Result<String, String> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    {
        let cache = state.d365_token.lock().await;
        if let Some((tok, exp)) = cache.as_ref() {
            if now < exp - 300 {
                return Ok(tok.clone());
            }
        }
    }

    let cfg = &state.config;
    let tenant = cfg.azure_tenant_id.as_deref().ok_or("AZURE_TENANT_ID not set")?;
    let client_id = cfg.azure_client_id.as_deref().ok_or("AZURE_CLIENT_ID not set")?;
    let secret = cfg.azure_client_secret.as_deref().ok_or("AZURE_CLIENT_SECRET not set")?;
    let base = cfg.d365_base_url.as_deref().ok_or("D365_BASE_URL not set")?;

    let url = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token");
    let scope = format!("{base}/.default");
    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", secret),
        ("scope", scope.as_str()),
    ];

    let resp = state
        .http
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;

    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token request returned {code}: {body}"));
    }

    let tr: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("could not parse token response: {e}"))?;

    let expires_at = now + if tr.expires_in > 0 { tr.expires_in } else { 3600 };
    {
        let mut cache = state.d365_token.lock().await;
        *cache = Some((tr.access_token.clone(), expires_at));
    }
    Ok(tr.access_token)
}

/// Run an OData GET against `/data/{entity}?{query}` and return the parsed JSON.
pub async fn odata_get(state: &AppState, entity: &str, query: &str) -> Result<Value, String> {
    let token = get_token(state).await?;
    let base = state.config.d365_base_url.as_deref().ok_or("D365_BASE_URL not set")?;
    let sep = if query.is_empty() { "" } else { "?" };
    let url = format!("{base}/data/{entity}{sep}{query}");

    let resp = state
        .http
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("OData-MaxVersion", "4.0")
        .header("OData-Version", "4.0")
        .send()
        .await
        .map_err(|e| format!("OData request failed: {e}"))?;

    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OData query returned {code}: {body}"));
    }

    resp.json::<Value>()
        .await
        .map_err(|e| format!("could not parse OData response: {e}"))
}
