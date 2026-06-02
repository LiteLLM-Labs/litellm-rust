use std::collections::HashMap;

use reqwest::Client;
use serde::Deserialize;

const MODEL_COST_MAP_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/refs/heads/litellm_internal_staging/model_prices_and_context_window.json";

static BACKUP_JSON: &str = include_str!("../model_prices_backup.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub litellm_provider: Option<String>,
    pub mode: Option<String>,
    pub max_tokens: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_cost_per_token: Option<f64>,
    pub output_cost_per_token: Option<f64>,
    pub supports_vision: Option<bool>,
    pub supports_function_calling: Option<bool>,
    pub supports_tool_choice: Option<bool>,
    pub supports_system_prompts: Option<bool>,
    pub supports_parallel_function_calling: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub type ModelCostMap = HashMap<String, ModelInfo>;

pub async fn load(http: &Client) -> ModelCostMap {
    let url = std::env::var("LITELLM_MODEL_COST_MAP_URL")
        .unwrap_or_else(|_| MODEL_COST_MAP_URL.to_owned());

    match fetch(http, &url).await {
        Ok(map) => {
            tracing::info!("Loaded model cost map from {url} ({} entries)", map.len());
            map
        }
        Err(e) => {
            tracing::warn!("Failed to fetch model cost map from {url}: {e} — using backup");
            load_backup()
        }
    }
}

async fn fetch(http: &Client, url: &str) -> Result<ModelCostMap, Box<dyn std::error::Error>> {
    let text = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let map = serde_json::from_str(&text)?;
    Ok(map)
}

fn load_backup() -> ModelCostMap {
    match serde_json::from_str(BACKUP_JSON) {
        Ok(map) => map,
        Err(e) => {
            tracing::error!("Failed to parse embedded backup model cost map: {e}");
            HashMap::new()
        }
    }
}
