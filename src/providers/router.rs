use std::{collections::HashMap, sync::Arc};

use crate::{
    errors::GatewayError,
    providers::transform::{ProviderRegistry, Transformation},
    proxy::config::GatewayConfig,
};

#[derive(Debug, Clone)]
pub struct Deployment {
    pub provider_id: String,
    pub upstream_model: String,
    pub api_base: String,
    pub api_key: String,
}

impl Deployment {
    pub fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.api_base.trim_end_matches('/'))
    }
}

pub struct Route {
    pub deployment: Deployment,
    pub handler: Arc<dyn Transformation>,
}

pub struct Router {
    routes: HashMap<String, Route>,
}

impl Router {
    pub fn from_config(
        config: &GatewayConfig,
        providers: &ProviderRegistry,
    ) -> Result<Self, GatewayError> {
        let mut routes = HashMap::with_capacity(config.model_list.len());

        for entry in &config.model_list {
            let model = &entry.litellm_params.model;
            let Some((provider_id, upstream_model)) = model.split_once('/') else {
                return Err(GatewayError::InvalidConfig(format!(
                    "model must include provider prefix (e.g. anthropic/...), got {model}"
                )));
            };
            if upstream_model.trim().is_empty() {
                return Err(GatewayError::InvalidConfig(format!(
                    "model missing name after provider prefix, got {model}"
                )));
            }

            let provider = providers.get(provider_id).ok_or_else(|| {
                GatewayError::InvalidConfig(format!("unsupported provider: {provider_id}"))
            })?;

            let api_key = entry.litellm_params.api_key.clone().ok_or_else(|| {
                GatewayError::InvalidConfig(format!(
                    "{} is missing litellm_params.api_key",
                    entry.model_name
                ))
            })?;

            routes.insert(
                entry.model_name.clone(),
                Route {
                    deployment: Deployment {
                        provider_id: provider_id.to_owned(),
                        upstream_model: upstream_model.to_owned(),
                        api_base: entry
                            .litellm_params
                            .api_base
                            .clone()
                            .unwrap_or_else(|| provider.default_api_base.clone()),
                        api_key,
                    },
                    handler: provider.handler,
                },
            );
        }

        Ok(Self { routes })
    }

    pub fn resolve(&self, model: &str) -> Result<&Route, GatewayError> {
        self.routes
            .get(model)
            .ok_or_else(|| GatewayError::UnknownModel(model.to_owned()))
    }
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("models", &self.routes.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Router;
    use crate::providers::{self, transform::ProviderRegistry};
    use crate::proxy::config::{GatewayConfig, LiteLlmParams, ModelEntry};

    #[test]
    fn resolves_model_to_upstream() {
        let mut providers = ProviderRegistry::new();
        providers::register_all(&mut providers);

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

        let router = Router::from_config(&config, &providers).unwrap();
        let route = router.resolve("claude").unwrap();
        assert_eq!(route.deployment.upstream_model, "claude-sonnet-4-5");
        assert_eq!(route.deployment.provider_id, "anthropic");
    }
}
