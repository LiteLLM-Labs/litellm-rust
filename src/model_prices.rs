use std::collections::HashMap;

use reqwest::Client;
use serde::{Deserialize, Deserializer};

use crate::sdk::codec::ir::Usage;

const MODEL_COST_MAP_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/refs/heads/litellm_internal_staging/model_prices_and_context_window.json";

static BACKUP_JSON: &str = include_str!("model_prices_backup.json");

fn deserialize_opt_u64<'de, D>(d: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => Ok(n.as_u64()),
        serde_json::Value::Null => Ok(None),
        _ => Ok(None),
    }
}

fn deserialize_opt_f64<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => Ok(n.as_f64()),
        serde_json::Value::Null => Ok(None),
        _ => Ok(None),
    }
}

fn deserialize_opt_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Ok(Some(s)),
        serde_json::Value::Null => Ok(None),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub litellm_provider: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    pub max_tokens: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    pub max_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64")]
    pub input_cost_per_token: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64")]
    pub output_cost_per_token: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64")]
    pub cache_read_input_token_cost: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64")]
    pub cache_creation_input_token_cost: Option<f64>,
    pub supports_vision: Option<bool>,
    pub supports_function_calling: Option<bool>,
    pub supports_tool_choice: Option<bool>,
    pub supports_system_prompts: Option<bool>,
    pub supports_parallel_function_calling: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ModelInfo {
    /// Estimated USD cost for a request's token usage, pricing cheap cache reads
    /// and premium cache writes separately from full-rate input. Falls back to
    /// Anthropic's published multipliers (read 0.1x, write 1.25x of input) when
    /// the cost map lacks explicit cache rates. `None` if base prices are unknown.
    pub fn compute_cost(&self, usage: &Usage) -> Option<f64> {
        let input = self.input_cost_per_token?;
        let output = self.output_cost_per_token?;
        let read = self.cache_read_input_token_cost.unwrap_or(input * 0.1);
        let create = self.cache_creation_input_token_cost.unwrap_or(input * 1.25);
        Some(
            usage.non_cached_input_tokens() as f64 * input
                + usage.cache_read_input_tokens as f64 * read
                + usage.cache_creation_input_tokens as f64 * create
                + usage.output_tokens as f64 * output,
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk::codec::ir::Usage;

    fn info(input: f64, output: f64) -> ModelInfo {
        ModelInfo {
            litellm_provider: None,
            mode: None,
            max_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            input_cost_per_token: Some(input),
            output_cost_per_token: Some(output),
            cache_read_input_token_cost: None,
            cache_creation_input_token_cost: None,
            supports_vision: None,
            supports_function_calling: None,
            supports_tool_choice: None,
            supports_system_prompts: None,
            supports_parallel_function_calling: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn cost_uses_cache_fallback_multipliers() {
        let m = info(3e-6, 15e-6);
        // 50 fresh input + 1000 cache reads (0.1x) + 10 output
        let usage = Usage {
            input_tokens: 1050,
            output_tokens: 10,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 1000,
        };
        let cost = m.compute_cost(&usage).unwrap();
        let expected = 50.0 * 3e-6 + 1000.0 * (3e-6 * 0.1) + 10.0 * 15e-6;
        assert!(
            (cost - expected).abs() < 1e-12,
            "got {cost}, want {expected}"
        );
    }

    #[test]
    fn cost_prefers_explicit_cache_rates() {
        let mut m = info(3e-6, 15e-6);
        m.cache_read_input_token_cost = Some(0.5e-6);
        let usage = Usage {
            input_tokens: 1000,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 1000,
        };
        // all input is cache read at the explicit 0.5e-6 rate
        let cost = m.compute_cost(&usage).unwrap();
        assert!((cost - 1000.0 * 0.5e-6).abs() < 1e-12);
    }

    #[test]
    fn cost_none_without_base_prices() {
        let mut m = info(3e-6, 15e-6);
        m.input_cost_per_token = None;
        assert!(m.compute_cost(&Usage::default()).is_none());
    }
}
