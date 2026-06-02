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

/// A single MCP server, matching LiteLLM's `mcp_servers.<name>` config block.
/// The server name is the map key in `GatewayConfig.mcp_servers`, so it is not
/// repeated here (LiteLLM has no `id` field).
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    pub url: String,

    #[serde(default)]
    pub transport: McpTransport,

    #[serde(default)]
    pub auth_type: McpAuthType,

    /// The credential for `auth_type`. LiteLLM also accepts `authentication_token`
    /// as an alias; when both are present serde keeps the last one parsed.
    #[serde(default, alias = "authentication_token")]
    pub auth_value: Option<String>,

    /// Headers always sent to the upstream server.
    #[serde(default)]
    pub static_headers: HashMap<String, String>,

    /// Names of inbound request headers to forward upstream (allowlist).
    #[serde(default)]
    pub extra_headers: Vec<String>,

    /// Accepted for LiteLLM compatibility; not used by the gateway.
    #[serde(default)]
    pub description: Option<String>,
}

/// Upstream MCP transport. Only `http` (streamable HTTP) is currently served;
/// `sse`/`stdio` parse but are rejected by `validate` with a clear message.
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
    if config.model_list.is_empty() && config.mcp_servers.is_empty() {
        return Err(GatewayError::InvalidConfig(
            "model_list or mcp_servers must contain at least one entry".to_owned(),
        ));
    }

    for entry in &config.model_list {
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

    // Map keys give uniqueness for free; validate each server's fields.
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
            McpAuthType::None => {}
            _ => {
                if server.auth_value.as_deref().unwrap_or("").is_empty() {
                    return Err(GatewayError::InvalidConfig(format!(
                        "{name}: auth_type '{}' requires auth_value",
                        server.auth_type.as_str()
                    )));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::expand_env_value;

    #[test]
    fn leaves_literal_values_untouched() {
        assert_eq!(expand_env_value("sk-test").unwrap(), "sk-test");
    }
}
