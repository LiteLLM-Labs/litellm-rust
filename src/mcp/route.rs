use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method},
    response::Response,
};

use crate::{
    db::managed_agents::mcp_credentials,
    errors::GatewayError,
    mcp::registry::{McpServer, McpServerRegistry},
    proxy::{
        auth::{
            identity::{identify_caller, CallerIdentity},
            master_key::require_master_key,
        },
        state::AppState,
    },
};

const SERVER_HEADER: &str = "x-litellm-mcp-server";

pub async fn streamable_http(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    let server_id = select_server_id(&state.mcp_servers, &headers, query.get("server"))?.to_owned();
    serve(&state, &server_id, method, headers, body).await
}

pub async fn streamable_http_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    serve(&state, &server_id, method, headers, body).await
}

/// Authorize the caller, build any per-user auth header, and forward the request.
async fn serve(
    state: &AppState,
    server_id: &str,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    let server = state.mcp_servers.resolve(server_id)?;
    let user_auth = authorize(state, server, server_id, &headers).await?;
    crate::mcp::upstream::forward_streamable_http(
        &state.http,
        server,
        method,
        &headers,
        body,
        user_auth,
    )
    .await
}

/// For shared servers, require the master key (admin). For BYOK servers, require
/// a valid user key and resolve that user's stored credential into an auth header.
async fn authorize(
    state: &AppState,
    server: &McpServer,
    server_id: &str,
    headers: &HeaderMap,
) -> Result<Option<(HeaderName, HeaderValue)>, GatewayError> {
    if !server.is_byok {
        require_master_key(headers, state.config.general_settings.master_key.as_deref())?;
        return Ok(None);
    }

    // BYOK: per-user credential injection. Config validation guarantees a master
    // key, database, and encryption key are present for BYOK servers.
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
    let user_id = match identity {
        CallerIdentity::User(user_id) => user_id,
        // The master key has no per-user credentials; reject rather than send an
        // unauthenticated request upstream.
        CallerIdentity::Admin => return Err(GatewayError::Unauthorized),
    };

    let credential = mcp_credentials::repository::resolve(db, enc_key, &user_id, server_id)
        .await?
        .ok_or_else(|| GatewayError::UserCredentialMissing(server_id.to_owned()))?;

    server.user_auth_header(&credential.value)
}

fn select_server_id<'a>(
    registry: &'a McpServerRegistry,
    headers: &'a HeaderMap,
    query_server: Option<&'a String>,
) -> Result<&'a str, GatewayError> {
    if let Some(server_id) = query_server {
        return Ok(server_id.as_str());
    }

    if let Some(server_id) = headers
        .get(SERVER_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(server_id);
    }

    registry
        .only_server_id()
        .ok_or(GatewayError::MissingMcpServer)
}
