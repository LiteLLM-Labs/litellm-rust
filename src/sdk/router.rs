use std::collections::HashMap;

use crate::{
    errors::GatewayError,
    proxy::config::GatewayConfig,
    sdk::{codec::WireFormat, providers::transform::ProviderRegistry},
};

#[derive(Debug, Clone)]
pub struct Deployment {
    pub provider_id: String,
    pub upstream_model: String,
    pub api_base: String,
    pub api_key: String,
    /// Wire format this deployment speaks upstream — picks the outbound codec.
    pub wire: WireFormat,
}

impl Deployment {
    /// Upstream URL for this deployment's wire format. Gemini encodes the model
    /// and the streaming variant in the path.
    pub fn upstream_url(&self, stream: bool) -> String {
        let base = self.api_base.trim_end_matches('/');
        match self.wire {
            WireFormat::AnthropicMessages => format!("{base}/v1/messages"),
            WireFormat::OpenAiChat => format!("{base}/v1/chat/completions"),
            WireFormat::OpenAiResponses => format!("{base}/v1/responses"),
            WireFormat::Gemini => {
                let method = if stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                };
                let model = &self.upstream_model;
                if stream {
                    format!("{base}/v1beta/models/{model}:{method}?alt=sse")
                } else {
                    format!("{base}/v1beta/models/{model}:{method}")
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Route {
    pub deployment: Deployment,
}

pub struct Router {
    routes: HashMap<String, Route>,
    /// Wildcard fallbacks keyed by their public prefix (e.g. `anthropic` for an
    /// `anthropic/*` route), so multiple providers can each declare one.
    wildcards: HashMap<String, Route>,
}

impl Router {
    pub fn from_config(
        config: &GatewayConfig,
        providers: &ProviderRegistry,
    ) -> Result<Self, GatewayError> {
        let mut routes = HashMap::with_capacity(config.model_list.len());
        let mut wildcards: HashMap<String, Route> = HashMap::new();

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

            let wire = match &entry.litellm_params.wire_api {
                Some(value) => WireFormat::parse(value).ok_or_else(|| {
                    GatewayError::InvalidConfig(format!("unknown wire_api: {value}"))
                })?,
                None => provider.default_wire,
            };

            let route = Route {
                deployment: Deployment {
                    provider_id: provider_id.to_owned(),
                    upstream_model: upstream_model.to_owned(),
                    api_base: entry
                        .litellm_params
                        .api_base
                        .clone()
                        .unwrap_or_else(|| provider.default_api_base.clone()),
                    api_key: entry.litellm_params.api_key.clone().unwrap_or_default(),
                    wire,
                },
            };

            if entry.model_name.ends_with("/*") && upstream_model == "*" {
                let prefix = entry.model_name.trim_end_matches("/*").to_owned();
                if wildcards.contains_key(&prefix) {
                    return Err(GatewayError::InvalidConfig(format!(
                        "duplicate wildcard route for prefix {prefix}"
                    )));
                }
                wildcards.insert(prefix, route);
            } else {
                routes.insert(entry.model_name.clone(), route);
            }
        }

        Ok(Self { routes, wildcards })
    }

    pub fn resolve(&self, model: &str) -> Result<Route, GatewayError> {
        if let Some(route) = self.routes.get(model) {
            tracing::debug!(
                model,
                upstream_model = %route.deployment.upstream_model,
                provider = %route.deployment.provider_id,
                "router: exact match"
            );
            return Ok(route.clone());
        }

        let prefix = model.split_once('/').map(|(p, _)| p).unwrap_or(model);
        let Some(route) = self.wildcards.get(prefix) else {
            return Err(GatewayError::UnknownModel(model.to_owned()));
        };
        let mut route = route.clone();
        route.deployment.upstream_model = passthrough_model(model, &route.deployment.provider_id);
        tracing::debug!(model, "router: wildcard match — stripped provider prefix");
        Ok(route)
    }

    /// Resolve with inbound-protocol context. Native Gemini requests carry a bare
    /// model name in the URL, so on an exact miss retry the `gemini/*` wildcard.
    pub fn resolve_wire(
        &self,
        inbound_wire: WireFormat,
        model: &str,
    ) -> Result<Route, GatewayError> {
        match self.resolve(model) {
            Err(GatewayError::UnknownModel(_))
                if inbound_wire == WireFormat::Gemini && !model.contains('/') =>
            {
                self.resolve(&format!("gemini/{model}"))
            }
            other => other,
        }
    }
}

fn passthrough_model(model: &str, provider_id: &str) -> String {
    model
        .strip_prefix(&format!("{provider_id}/"))
        .unwrap_or(model)
        .to_owned()
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("models", &self.routes.keys().collect::<Vec<_>>())
            .field("wildcards", &self.wildcards.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Router;
    use crate::proxy::config::{GatewayConfig, LiteLlmParams, ModelEntry};
    use crate::sdk::providers::{self, transform::ProviderRegistry};

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
                    wire_api: None,
                    extra: Default::default(),
                },
            }],
            mcp_servers: HashMap::new(),
            general_settings: Default::default(),
            agents: Vec::new(),
        };

        let router = Router::from_config(&config, &providers).unwrap();
        let route = router.resolve("claude").unwrap();
        assert_eq!(route.deployment.upstream_model, "claude-sonnet-4-5");
        assert_eq!(route.deployment.provider_id, "anthropic");
    }

    #[test]
    fn resolves_wildcard_model_to_anthropic_passthrough() {
        let mut providers = ProviderRegistry::new();
        providers::register_all(&mut providers);

        let config = GatewayConfig {
            model_list: vec![ModelEntry {
                model_name: "anthropic/*".to_owned(),
                litellm_params: LiteLlmParams {
                    model: "anthropic/*".to_owned(),
                    api_key: Some("sk".to_owned()),
                    api_base: None,
                    wire_api: None,
                    extra: Default::default(),
                },
            }],
            mcp_servers: HashMap::new(),
            general_settings: Default::default(),
            agents: Vec::new(),
        };

        let router = Router::from_config(&config, &providers).unwrap();
        let route = router.resolve("anthropic/claude-opus-4-8").unwrap();
        assert_eq!(route.deployment.provider_id, "anthropic");
        assert_eq!(route.deployment.upstream_model, "claude-opus-4-8");
    }

    #[test]
    fn strips_provider_prefix_from_wildcard_model() {
        let mut providers = ProviderRegistry::new();
        providers::register_all(&mut providers);

        let config = GatewayConfig {
            model_list: vec![ModelEntry {
                model_name: "anthropic/*".to_owned(),
                litellm_params: LiteLlmParams {
                    model: "anthropic/*".to_owned(),
                    api_key: Some("sk".to_owned()),
                    api_base: None,
                    wire_api: None,
                    extra: Default::default(),
                },
            }],
            mcp_servers: HashMap::new(),
            general_settings: Default::default(),
            agents: Vec::new(),
        };

        let router = Router::from_config(&config, &providers).unwrap();
        let route = router.resolve("anthropic/claude-opus-4-8").unwrap();
        assert_eq!(route.deployment.upstream_model, "claude-opus-4-8");
    }

    #[test]
    fn exact_route_takes_precedence_over_wildcard() {
        let mut providers = ProviderRegistry::new();
        providers::register_all(&mut providers);

        let config = GatewayConfig {
            model_list: vec![
                ModelEntry {
                    model_name: "claude".to_owned(),
                    litellm_params: LiteLlmParams {
                        model: "anthropic/claude-sonnet-4-5".to_owned(),
                        api_key: Some("sk".to_owned()),
                        api_base: None,
                        wire_api: None,
                        extra: Default::default(),
                    },
                },
                ModelEntry {
                    model_name: "anthropic/*".to_owned(),
                    litellm_params: LiteLlmParams {
                        model: "anthropic/*".to_owned(),
                        api_key: Some("sk".to_owned()),
                        api_base: None,
                        wire_api: None,
                        extra: Default::default(),
                    },
                },
            ],
            mcp_servers: HashMap::new(),
            general_settings: Default::default(),
            agents: Vec::new(),
        };

        let router = Router::from_config(&config, &providers).unwrap();
        let route = router.resolve("claude").unwrap();
        assert_eq!(route.deployment.upstream_model, "claude-sonnet-4-5");
    }
}
