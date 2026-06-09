use std::collections::HashMap;

use serde::Deserialize;

use crate::agents::config::{AgentDefinition, E2bSandboxParams};
use crate::proxy::mcp_config::McpServerEntry;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub model_list: Vec<ModelEntry>,

    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerEntry>,

    #[serde(default)]
    pub general_settings: GeneralSettings,

    #[serde(default)]
    pub agents: Vec<AgentDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeneralSettings {
    pub master_key: Option<String>,
    pub database_url: Option<String>,
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
    /// Override the provider's default wire format: `chat` | `responses` |
    /// `gemini` | `anthropic`. When absent, the provider id's default is used.
    #[serde(default)]
    pub wire_api: Option<String>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}
