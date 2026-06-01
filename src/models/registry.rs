use std::collections::HashMap;

use crate::{
    app::errors::GatewayError, config::schema::GatewayConfig, models::deployment::Deployment,
};

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    deployments: HashMap<String, Deployment>,
}

impl ModelRegistry {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, GatewayError> {
        let mut deployments = HashMap::with_capacity(config.model_list.len());

        for entry in &config.model_list {
            let deployment = Deployment::new(
                entry.litellm_params.model.clone(),
                entry.litellm_params.api_base.clone(),
                entry.litellm_params.api_key.clone().ok_or_else(|| {
                    GatewayError::InvalidConfig(format!(
                        "{} is missing litellm_params.api_key",
                        entry.model_name
                    ))
                })?,
            )?;

            deployments.insert(entry.model_name.clone(), deployment);
        }

        Ok(Self { deployments })
    }

    pub fn resolve(&self, model: &str) -> Result<&Deployment, GatewayError> {
        self.deployments
            .get(model)
            .ok_or_else(|| GatewayError::UnknownModel(model.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::schema::{GatewayConfig, LiteLlmParams, ModelEntry};

    use super::ModelRegistry;

    #[test]
    fn strips_anthropic_prefix_for_upstream_model() {
        let config = GatewayConfig {
            model_list: vec![ModelEntry {
                model_name: "claude".to_owned(),
                litellm_params: LiteLlmParams {
                    model: "anthropic/claude-sonnet-4-5".to_owned(),
                    api_key: Some("sk".to_owned()),
                    api_base: None,
                    extra: Default::default(),
                },
            }],
            general_settings: Default::default(),
        };

        let registry = ModelRegistry::from_config(&config).unwrap();
        let deployment = registry.resolve("claude").unwrap();
        assert_eq!(deployment.upstream_model, "claude-sonnet-4-5");
        assert_eq!(
            deployment.provider,
            crate::ai_gateway::provider::LlmProvider::Anthropic
        );
    }
}
