//! Admin endpoints for minting per-user identities and API keys.
//!
//! Both require the master key (enforced via [`crate::http::managed_agents::db`]).

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};

use crate::{
    db::managed_agents::users::{
        repository,
        schema::{GenerateKeyRequest, GenerateKeyResponse, NewUserRequest, NewUserResponse},
    },
    errors::GatewayError,
    http::managed_agents::db,
    proxy::state::AppState,
};

/// `POST /user/new` — create a user, minting an API key unless `auto_create_key`
/// is false.
pub async fn new_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<NewUserRequest>,
) -> Result<Json<NewUserResponse>, GatewayError> {
    let pool = db(&state, &headers)?;
    let response = repository::create_user(pool, input).await?;
    Ok(Json(response))
}

/// `POST /key/generate` — mint a new API key for a user (created if absent).
pub async fn generate_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<GenerateKeyRequest>,
) -> Result<Json<GenerateKeyResponse>, GatewayError> {
    let pool = db(&state, &headers)?;
    let response = repository::generate_key(pool, input.user_id, input.key_alias).await?;
    Ok(Json(response))
}
