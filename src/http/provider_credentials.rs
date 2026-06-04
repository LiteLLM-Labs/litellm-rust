use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};
use serde::{Deserialize, Serialize};

use crate::{
    db::credentials,
    errors::GatewayError,
    proxy::{
        auth::master_key::require_master_key,
        provider_credentials::{
            self, credential_name, ANTHROPIC_PROVIDER_ID, DEFAULT_ANTHROPIC_BASE_URL,
        },
        state::AppState,
    },
};

#[derive(Debug, Serialize)]
pub struct ProvidersResponse {
    pub available_providers: Vec<AvailableProvider>,
    pub connected_providers: Vec<ConnectedProvider>,
}

#[derive(Debug, Serialize)]
pub struct AvailableProvider {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_base_url: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectedProvider {
    pub id: String,
    pub name: String,
    pub api_base: String,
    pub masked_api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveProviderRequest {
    pub api_key: String,
    pub api_base: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteProviderResponse {
    pub ok: bool,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ProvidersResponse>, GatewayError> {
    require_admin(&state, &headers)?;
    Ok(Json(response(&state).await?))
}

pub async fn save_anthropic(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<SaveProviderRequest>,
) -> Result<Json<ProvidersResponse>, GatewayError> {
    require_admin(&state, &headers)?;
    let pool = state.db.as_ref().ok_or(GatewayError::MissingDatabase)?;
    let api_key = required(input.api_key, "api_key")?;
    let api_base = required(input.api_base, "api_base")?;
    provider_credentials::save_anthropic(pool, &state.config, &api_key, &api_base).await?;
    Ok(Json(response(&state).await?))
}

pub async fn delete_anthropic(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<DeleteProviderResponse>, GatewayError> {
    require_admin(&state, &headers)?;
    let pool = state.db.as_ref().ok_or(GatewayError::MissingDatabase)?;
    Ok(Json(DeleteProviderResponse {
        ok: credentials::delete_by_name(pool, &credential_name(ANTHROPIC_PROVIDER_ID)).await?,
    }))
}

async fn response(state: &AppState) -> Result<ProvidersResponse, GatewayError> {
    Ok(ProvidersResponse {
        available_providers: vec![AvailableProvider {
            id: ANTHROPIC_PROVIDER_ID.to_owned(),
            name: "Anthropic".to_owned(),
            description: "Claude models through the Anthropic Messages API".to_owned(),
            default_base_url: DEFAULT_ANTHROPIC_BASE_URL.to_owned(),
        }],
        connected_providers: connected_anthropic(state).await?.into_iter().collect(),
    })
}

async fn connected_anthropic(state: &AppState) -> Result<Option<ConnectedProvider>, GatewayError> {
    let Some(pool) = state.db.as_ref() else {
        return Ok(None);
    };
    let Some(credential) =
        provider_credentials::load(pool, &state.config, ANTHROPIC_PROVIDER_ID).await?
    else {
        return Ok(None);
    };
    Ok(Some(ConnectedProvider {
        id: ANTHROPIC_PROVIDER_ID.to_owned(),
        name: "Anthropic".to_owned(),
        api_base: credential.api_base,
        masked_api_key: provider_credentials::mask_api_key(&credential.api_key),
    }))
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), GatewayError> {
    require_master_key(headers, state.config.general_settings.master_key.as_deref())
}

fn required(value: String, field: &str) -> Result<String, GatewayError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(GatewayError::InvalidJsonMessage(format!(
            "{field} is required"
        )));
    }
    Ok(trimmed.to_owned())
}
