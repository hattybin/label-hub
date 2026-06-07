//! Small helpers (token/secret generation).

use uuid::Uuid;

/// A long opaque token (~64 hex chars) from two v4 UUIDs. Adequate for per-node
/// tokens / enrollment tokens / inbound secrets at this scale.
pub fn random_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
