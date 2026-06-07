//! Operator authentication for the dashboard.
//!
//! In production the dashboard runs behind Azure Container Apps / App Service
//! **EasyAuth**, which injects `X-MS-CLIENT-PRINCIPAL` (base64 JSON of the signed-in
//! user's claims, including Entra **app roles**). We parse that header; no OIDC code
//! is needed. For local dev, `DEV_ADMIN` short-circuits to an admin identity.

use axum::http::HeaderMap;
use base64::Engine;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct Principal {
    pub email: String,
    pub roles: Vec<String>,
}

impl Principal {
    pub fn is_admin(&self) -> bool {
        self.roles.iter().any(|r| r.eq_ignore_ascii_case("admin") || r.eq_ignore_ascii_case("Label.Admin"))
    }
}

#[derive(Deserialize)]
struct ClientPrincipal {
    #[serde(default, rename = "claims")]
    claims: Vec<Claim>,
}

#[derive(Deserialize)]
struct Claim {
    #[serde(default, rename = "typ")]
    typ: String,
    #[serde(default, rename = "val")]
    val: String,
}

/// Extract the operator identity, or None if unauthenticated.
pub fn principal(state: &AppState, headers: &HeaderMap) -> Option<Principal> {
    if let Some(raw) = headers.get("x-ms-client-principal").and_then(|v| v.to_str().ok()) {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw) {
            if let Ok(cp) = serde_json::from_slice::<ClientPrincipal>(&bytes) {
                let mut email = String::new();
                let mut roles = Vec::new();
                for c in cp.claims {
                    let t = c.typ.to_ascii_lowercase();
                    if t.ends_with("emailaddress") || t == "preferred_username" || t == "email" || t.ends_with("/upn") {
                        if email.is_empty() {
                            email = c.val;
                        }
                    } else if t == "roles" || t.ends_with("/role") || t == "role" {
                        roles.push(c.val);
                    }
                }
                return Some(Principal { email, roles });
            }
        }
    }
    // Dev fallback.
    state.cfg.dev_admin.as_ref().map(|email| Principal {
        email: email.clone(),
        roles: vec!["admin".into()],
    })
}

/// Sites an operator may see. `None` = all sites (admin).
pub async fn allowed_sites(state: &AppState, p: &Principal) -> Option<Vec<String>> {
    if p.is_admin() {
        return None;
    }
    let rows: Vec<(String,)> = sqlx::query_as("SELECT site FROM user_sites WHERE user_email = $1")
        .bind(&p.email)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Some(rows.into_iter().map(|r| r.0).collect())
}
