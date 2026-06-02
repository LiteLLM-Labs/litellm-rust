use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

use crate::errors::GatewayError;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub model_list: Vec<ModelEntry>,

    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerEntry>,

    #[serde(default)]
    pub general_settings: GeneralSettings,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeneralSettings {
    pub master_key: Option<String>,
    pub database_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub model_name: String,
    pub litellm_params: LiteLlmParams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiteLlmParams {
    pub model: String,
    pub api_key: Option<String>,
    pub api_base: Option<String>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    pub url: String,

    #[serde(default)]
    pub transport: McpTransport,

    #[serde(default)]
    pub auth_type: McpAuthType,

    #[serde(default, alias = "authentication_token")]
    pub auth_value: Option<String>,

    #[serde(default)]
    pub static_headers: HashMap<String, String>,

    #[serde(default)]
    pub extra_headers: Vec<String>,

    #[serde(default)]
    pub description: Option<String>,

    /// Bring-your-own-key: each user supplies their own upstream credential
    /// (stored per-user, injected at request time) instead of a shared
    /// `auth_value`. Mirrors LiteLLM's `is_byok`.
    #[serde(default)]
    pub is_byok: bool,

    /// Human-readable hints shown to users about what credential to provide.
    #[serde(default)]
    pub byok_description: Vec<String>,

    /// Link to instructions for obtaining the credential.
    #[serde(default)]
    pub byok_api_key_help_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Http,
    Sse,
    Stdio,
}

/// Upstream auth scheme, mirroring LiteLLM's `MCPAuth`. The static-header
/// variants are implemented; `oauth2`/`oauth2_token_exchange`/`aws_sigv4` parse
/// but are rejected by `validate` until implemented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthType {
    #[default]
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Token,
    Oauth2,
    Oauth2TokenExchange,
    AwsSigv4,
}

impl McpAuthType {
    /// The wire/config string for this variant, for error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ApiKey => "api_key",
            Self::BearerToken => "bearer_token",
            Self::Basic => "basic",
            Self::Authorization => "authorization",
            Self::Token => "token",
            Self::Oauth2 => "oauth2",
            Self::Oauth2TokenExchange => "oauth2_token_exchange",
            Self::AwsSigv4 => "aws_sigv4",
        }
    }
}

pub fn load_config(path: &Path) -> Result<GatewayConfig, GatewayError> {
    let raw = fs::read_to_string(path)?;
    let mut config: GatewayConfig = serde_yaml::from_str(&raw).map_err(|error| {
        // `mcp_servers` changed from a list to a dict keyed by server name.
        // serde reports this as an "invalid type: sequence" error; translate it
        // into actionable guidance for anyone upgrading an old config.
        if is_mcp_sequence_error(&raw, &error) {
            GatewayError::InvalidConfig(
                "mcp_servers is now a dict keyed by server name (was a list). \
                 See docs/mcp.md for the new format."
                    .to_owned(),
            )
        } else {
            GatewayError::from(error)
        }
    })?;
    expand_env(&mut config)?;
    validate(&config)?;
    Ok(config)
}

/// True when the parse error is the old list-shaped `mcp_servers` hitting the
/// new map type. We require both a sequence-type error and a top-level
/// `mcp_servers:` list marker so unrelated sequence errors pass through.
fn is_mcp_sequence_error(raw: &str, error: &serde_yaml::Error) -> bool {
    let message = error.to_string();
    if !message.contains("invalid type: sequence") {
        return false;
    }
    let mut in_mcp = false;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("mcp_servers:") {
            // Inline list, e.g. `mcp_servers: []` or `[...]`.
            if rest.trim_start().starts_with('[') {
                return true;
            }
            in_mcp = true;
            continue;
        }
        if in_mcp {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // A block-list item directly under mcp_servers means the old shape.
            if line.starts_with(char::is_whitespace) && trimmed.starts_with("- ") {
                return true;
            }
            // Any other non-indented key ends the mcp_servers block.
            if !line.starts_with(char::is_whitespace) {
                in_mcp = false;
            }
        }
    }
    false
}

pub fn expand_env_value(value: &str) -> Result<String, GatewayError> {
    let Some(name) = value.strip_prefix("os.environ/") else {
        return Ok(value.to_owned());
    };

    std::env::var(name).map_err(|_| {
        GatewayError::InvalidConfig(format!("environment variable {name} is required"))
    })
}

fn expand_env(config: &mut GatewayConfig) -> Result<(), GatewayError> {
    if let Some(master_key) = config.general_settings.master_key.as_deref() {
        config.general_settings.master_key = Some(expand_env_value(master_key)?);
    }
    if let Some(database_url) = config.general_settings.database_url.as_deref() {
        config.general_settings.database_url = Some(expand_env_value(database_url)?);
    }

    for entry in &mut config.model_list {
        if let Some(api_key) = entry.litellm_params.api_key.as_deref() {
            entry.litellm_params.api_key = Some(expand_env_value(api_key)?);
        }
        if let Some(api_base) = entry.litellm_params.api_base.as_deref() {
            entry.litellm_params.api_base = Some(expand_env_value(api_base)?);
        }
    }

    for server in config.mcp_servers.values_mut() {
        server.url = expand_env_value(&server.url)?;
        if let Some(auth_value) = server.auth_value.as_deref() {
            server.auth_value = Some(expand_env_value(auth_value)?);
        }
        for value in server.static_headers.values_mut() {
            *value = expand_env_value(value)?;
        }
    }

    Ok(())
}

fn validate(config: &GatewayConfig) -> Result<(), GatewayError> {
    validate_required_surface(config)?;
    validate_model_entries(&config.model_list)?;
    validate_mcp_servers(config)?;
    Ok(())
}

fn validate_required_surface(config: &GatewayConfig) -> Result<(), GatewayError> {
    if config.model_list.is_empty()
        && config.mcp_servers.is_empty()
        && config.general_settings.database_url.is_none()
    {
        return Err(GatewayError::InvalidConfig(
            "model_list, mcp_servers, or general_settings.database_url must contain at least one entry".to_owned(),
        ));
    }
    Ok(())
}

fn validate_model_entries(entries: &[ModelEntry]) -> Result<(), GatewayError> {
    for entry in entries {
        if entry.model_name.trim().is_empty() {
            return Err(GatewayError::InvalidConfig(
                "model_name cannot be empty".to_owned(),
            ));
        }

        if !entry.litellm_params.model.contains('/') {
            return Err(GatewayError::InvalidConfig(format!(
                "model must include provider prefix (e.g. anthropic/...), got {}",
                entry.litellm_params.model
            )));
        }

        if entry
            .litellm_params
            .api_key
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            return Err(GatewayError::InvalidConfig(format!(
                "{} is missing litellm_params.api_key",
                entry.model_name
            )));
        }
    }
    Ok(())
}

fn validate_mcp_servers(config: &GatewayConfig) -> Result<(), GatewayError> {
    let has_master_key = config.general_settings.master_key.is_some();
    let has_database = config.general_settings.database_url.is_some();

    for (name, server) in &config.mcp_servers {
        if name.trim().is_empty() {
            return Err(GatewayError::InvalidConfig(
                "mcp server name cannot be empty".to_owned(),
            ));
        }
        if server.url.trim().is_empty() {
            return Err(GatewayError::InvalidConfig(format!(
                "{name} is missing mcp_servers.url"
            )));
        }
        if server.transport != McpTransport::Http {
            return Err(GatewayError::InvalidConfig(format!(
                "{name}: only 'http' transport is supported"
            )));
        }
        match server.auth_type {
            McpAuthType::Oauth2 | McpAuthType::Oauth2TokenExchange | McpAuthType::AwsSigv4 => {
                return Err(GatewayError::InvalidConfig(format!(
                    "{name}: auth_type '{}' not yet supported",
                    server.auth_type.as_str()
                )));
            }
            McpAuthType::None => {
                if server.is_byok {
                    return Err(GatewayError::InvalidConfig(format!(
                        "{name}: is_byok requires an auth_type to inject the user credential (e.g. bearer_token)"
                    )));
                }
            }
            _ => {
                if server.is_byok {
                    // BYOK supplies the credential per-user at request time, so a
                    // shared auth_value must not be set.
                    if server.auth_value.is_some() {
                        return Err(GatewayError::InvalidConfig(format!(
                            "{name}: is_byok cannot be combined with a shared auth_value"
                        )));
                    }
                } else if server.auth_value.as_deref().unwrap_or("").is_empty() {
                    return Err(GatewayError::InvalidConfig(format!(
                        "{name}: auth_type '{}' requires auth_value",
                        server.auth_type.as_str()
                    )));
                }
            }
        }

        if server.is_byok {
            if !has_master_key {
                return Err(GatewayError::InvalidConfig(format!(
                    "{name}: is_byok requires general_settings.master_key (used to authenticate users and encrypt credentials)"
                )));
            }
            if !has_database {
                return Err(GatewayError::InvalidConfig(format!(
                    "{name}: is_byok requires general_settings.database_url to store per-user credentials"
                )));
            }
        }
    }
    Ok(())
}
