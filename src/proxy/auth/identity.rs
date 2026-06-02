//! Caller identification: distinguishes the admin master key from per-user API
//! keys stored in the database.

use axum::http::HeaderMap;
use sqlx::PgPool;

use crate::{
    db::managed_agents::users::repository as users, errors::GatewayError,
    proxy::auth::master_key::presented_key,
};

/// Who is making a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerIdentity {
    /// Presented the configured master key.
    Admin,
    /// Presented a valid per-user API key.
    User(String),
}

/// Identify the caller from request headers.
///
/// Order: the master key wins (admin), then a database-backed user key. Returns
/// [`GatewayError::Unauthorized`] when neither matches.
pub async fn identify_caller(
    headers: &HeaderMap,
    master_key: Option<&str>,
    db: Option<&PgPool>,
) -> Result<CallerIdentity, GatewayError> {
    let presented = presented_key(headers).unwrap_or("");

    if let Some(master_key) = master_key {
        if presented == master_key {
            return Ok(CallerIdentity::Admin);
        }
    }

    if let Some(pool) = db {
        if let Some(user_id) = users::resolve_user(pool, presented).await? {
            return Ok(CallerIdentity::User(user_id));
        }
    }

    Err(GatewayError::Unauthorized)
}
