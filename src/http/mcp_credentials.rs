//! Per-user MCP credential endpoints (LiteLLM-compatible surface).
//!
//! All endpoints authenticate the caller as a *user* (a database-backed API
//! key), not the admin master key. Stored secrets are encrypted at rest.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    db::managed_agents::{
        mcp_credentials::{
            repository,
            schema::{
                McpOAuthUserCredentialRequest, McpUserCredentialRequest, McpUserCredentialStatus,
            },
        },
        now_ms,
    },
    errors::GatewayError,
    proxy::{
        auth::identity::{identify_caller, CallerIdentity},
        state::AppState,
    },
};

/// `POST /v1/mcp/server/{server_id}/user-credential` — store a static/BYOK token.
pub async fn put_static(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<McpUserCredentialRequest>,
) -> Result<Json<Value>, GatewayError> {
    let ctx = UserCtx::resolve(&state, &headers).await?;
    if body.save {
        repository::upsert_static(
            ctx.db,
            ctx.enc_key,
            &ctx.user_id,
            &server_id,
            &body.credential,
        )
        .await?;
    }
    Ok(Json(json!({ "server_id": server_id, "saved": body.save })))
}

/// `POST /v1/mcp/server/{server_id}/oauth-user-credential` — store an OAuth2 token set.
pub async fn put_oauth(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<McpOAuthUserCredentialRequest>,
) -> Result<Json<Value>, GatewayError> {
    let ctx = UserCtx::resolve(&state, &headers).await?;
    let expires_at = body.expires_in.map(|seconds| now_ms() + seconds * 1000);
    let scopes = body.scopes.as_ref().map(|scopes| scopes.join(" "));
    repository::upsert_oauth(
        ctx.db,
        ctx.enc_key,
        &ctx.user_id,
        &server_id,
        &body.access_token,
        body.refresh_token.as_deref(),
        expires_at,
        scopes.as_deref(),
    )
    .await?;
    Ok(Json(
        json!({ "server_id": server_id, "credential_type": "oauth" }),
    ))
}

/// `GET /v1/mcp/server/{server_id}/user-credential` and the OAuth status route.
pub async fn status(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<McpUserCredentialStatus>, GatewayError> {
    let ctx = UserCtx::resolve(&state, &headers).await?;
    let status = repository::status(ctx.db, &ctx.user_id, &server_id)
        .await?
        .unwrap_or(McpUserCredentialStatus {
            server_id,
            credential_type: "none".to_owned(),
            has_credential: false,
            expires_at: None,
        });
    Ok(Json(status))
}

/// `DELETE /v1/mcp/server/{server_id}/user-credential` (and oauth variant).
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, GatewayError> {
    let ctx = UserCtx::resolve(&state, &headers).await?;
    let deleted = repository::delete(ctx.db, &ctx.user_id, &server_id).await?;
    Ok(Json(json!({ "server_id": server_id, "deleted": deleted })))
}

/// `GET /v1/mcp/user-credentials` — list all of the caller's stored credentials.
pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<McpUserCredentialStatus>>, GatewayError> {
    let ctx = UserCtx::resolve(&state, &headers).await?;
    let items = repository::list_for_user(ctx.db, &ctx.user_id).await?;
    Ok(Json(items))
}

/// Authenticated-user request context: the resolved user plus the DB pool and
/// encryption key needed to read/write their credentials.
struct UserCtx<'a> {
    user_id: String,
    db: &'a PgPool,
    enc_key: &'a [u8; 32],
}

impl<'a> UserCtx<'a> {
    async fn resolve(state: &'a AppState, headers: &HeaderMap) -> Result<Self, GatewayError> {
        let db = state.db.as_ref().ok_or(GatewayError::MissingDatabase)?;
        let enc_key = state
            .enc_key
            .as_ref()
            .ok_or_else(|| GatewayError::Crypto("encryption key unavailable".to_owned()))?;
        let identity = identify_caller(
            headers,
            state.config.general_settings.master_key.as_deref(),
            Some(db),
        )
        .await?;
        match identity {
            CallerIdentity::User(user_id) => Ok(Self {
                user_id,
                db,
                enc_key,
            }),
            // These endpoints manage per-user credentials; the admin key is not a user.
            CallerIdentity::Admin => Err(GatewayError::Unauthorized),
        }
    }
}
