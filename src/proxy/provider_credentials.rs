use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    db::credentials,
    errors::GatewayError,
    proxy::{config::GatewayConfig, credential_crypto},
};

pub const ANTHROPIC_PROVIDER_ID: &str = "anthropic";
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone)]
pub struct ProviderCredential {
    pub api_key: String,
    pub api_base: String,
}

pub fn credential_name(provider_id: &str) -> String {
    format!("provider:{provider_id}")
}

pub async fn save_anthropic(
    pool: &PgPool,
    config: &GatewayConfig,
    api_key: &str,
    api_base: &str,
) -> Result<(), GatewayError> {
    let key = credential_crypto::encryption_key(config.general_settings.master_key.as_deref())?;
    let values = json!({
        "api_key": credential_crypto::encrypt_value(api_key, &key)?,
        "api_base": credential_crypto::encrypt_value(api_base, &key)?,
    });
    let info = json!({
        "custom_llm_provider": ANTHROPIC_PROVIDER_ID,
        "source": "litellm-rust-ui",
    });
    credentials::upsert(
        pool,
        &credential_name(ANTHROPIC_PROVIDER_ID),
        values,
        info,
        "ui",
    )
    .await
}

pub async fn load(
    pool: &PgPool,
    config: &GatewayConfig,
    provider_id: &str,
) -> Result<Option<ProviderCredential>, GatewayError> {
    let Some(row) = credentials::get_by_name(pool, &credential_name(provider_id)).await? else {
        return Ok(None);
    };
    let key = credential_crypto::encryption_key(config.general_settings.master_key.as_deref())?;
    let values = row.credential_values.as_object().ok_or_else(|| {
        GatewayError::InvalidConfig("credential_values must be an object".to_owned())
    })?;
    Ok(Some(ProviderCredential {
        api_key: decrypt_field(values, "api_key", &key)?,
        api_base: decrypt_field(values, "api_base", &key)?,
    }))
}

pub fn mask_api_key(api_key: &str) -> String {
    let trimmed = api_key.trim();
    if trimmed.len() <= 12 {
        return "Configured".to_owned();
    }
    format!("{}...{}", &trimmed[..7], &trimmed[trimmed.len() - 4..])
}

fn decrypt_field(
    values: &serde_json::Map<String, Value>,
    field: &str,
    key: &str,
) -> Result<String, GatewayError> {
    let encrypted = values
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::InvalidConfig(format!("credential is missing {field}")))?;
    credential_crypto::decrypt_value(encrypted, key)
}
