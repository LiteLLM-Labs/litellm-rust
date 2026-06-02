use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

use crate::{
    agents::config::{validate_agents, AgentDefinition, E2bSandboxParams},
    errors::GatewayError,
};

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub model_list: Vec<ModelEntry>,

    #[serde(default)]
    pub mcp_servers: Vec<McpServerEntry>,

    #[serde(default)]
    pub general_settings: GeneralSettings,

    #[serde(default)]
    pub agents: Vec<AgentDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeneralSettings {
    pub master_key: Option<String>,
    pub sandbox_choice: Option<String>,
    #[serde(default)]
    pub e2b_sandbox_params: E2bSandboxParams,
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
    pub id: String,
    pub url: String,
    pub api_key: Option<String>,

    #[serde(default)]
    pub headers: HashMap<String, String>,
}

pub fn load_config(path: &Path) -> Result<GatewayConfig, GatewayError> {
    let raw = fs::read_to_string(path)?;
    let mut config: GatewayConfig = serde_yaml::from_str(&raw)?;
    expand_env(&mut config)?;
    validate(&config)?;
    Ok(config)
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

    for server in &mut config.mcp_servers {
        server.url = expand_env_value(&server.url)?;
        if let Some(api_key) = server.api_key.as_deref() {
            server.api_key = Some(expand_env_value(api_key)?);
        }
        for value in server.headers.values_mut() {
            *value = expand_env_value(value)?;
        }
    }

    if let Some(api_key) = config
        .general_settings
        .e2b_sandbox_params
        .e2b_api_key
        .as_deref()
    {
        config.general_settings.e2b_sandbox_params.e2b_api_key = Some(expand_env_value(api_key)?);
    }
    for value in config.general_settings.e2b_sandbox_params.envs.values_mut() {
        *value = expand_env_value(value)?;
    }

    Ok(())
}

fn validate(config: &GatewayConfig) -> Result<(), GatewayError> {
    if config.model_list.is_empty() && config.mcp_servers.is_empty() && config.agents.is_empty() {
        return Err(GatewayError::InvalidConfig(
            "config must contain at least one model, mcp server, or agent".to_owned(),
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

    let mut mcp_ids = std::collections::HashSet::with_capacity(config.mcp_servers.len());
    for server in &config.mcp_servers {
        if server.id.trim().is_empty() {
            return Err(GatewayError::InvalidConfig(
                "mcp_servers.id cannot be empty".to_owned(),
            ));
        }
        if !mcp_ids.insert(server.id.as_str()) {
            return Err(GatewayError::InvalidConfig(format!(
                "duplicate mcp server id: {}",
                server.id
            )));
        }
        if server.url.trim().is_empty() {
            return Err(GatewayError::InvalidConfig(format!(
                "{} is missing mcp_servers.url",
                server.id
            )));
        }
    }

    validate_agents(
        &config.agents,
        config.general_settings.sandbox_choice.as_deref(),
        &config.general_settings.e2b_sandbox_params,
    )?;

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
