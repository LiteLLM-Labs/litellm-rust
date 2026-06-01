use crate::ai_gateway::provider::{get_custom_llm_provider, LlmProvider};

#[derive(Debug, Clone)]
pub struct Deployment {
    pub provider: LlmProvider,
    pub upstream_model: String,
    pub api_base: String,
    pub api_key: String,
}

impl Deployment {
    pub fn new(
        litellm_model: String,
        api_base: Option<String>,
        api_key: String,
    ) -> Result<Self, crate::app::errors::GatewayError> {
        let provider_model = get_custom_llm_provider(&litellm_model)?;

        Ok(Self {
            provider: provider_model.provider,
            upstream_model: provider_model.model,
            api_base: api_base.unwrap_or_else(|| "https://api.anthropic.com".to_owned()),
            api_key,
        })
    }

    pub fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.api_base.trim_end_matches('/'))
    }
}
