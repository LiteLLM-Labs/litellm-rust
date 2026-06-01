use crate::app::errors::GatewayError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModel {
    pub provider: LlmProvider,
    pub model: String,
}

pub fn get_custom_llm_provider(model: &str) -> Result<ProviderModel, GatewayError> {
    let Some((provider, upstream_model)) = model.split_once('/') else {
        return Err(GatewayError::InvalidConfig(format!(
            "model must include provider prefix, got {model}"
        )));
    };

    let provider = match provider {
        "anthropic" => LlmProvider::Anthropic,
        other => {
            return Err(GatewayError::InvalidConfig(format!(
                "unsupported provider {other}; v0 supports anthropic only"
            )))
        }
    };

    if upstream_model.trim().is_empty() {
        return Err(GatewayError::InvalidConfig(format!(
            "model must include provider model after prefix, got {model}"
        )));
    }

    Ok(ProviderModel {
        provider,
        model: upstream_model.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{get_custom_llm_provider, LlmProvider};

    #[test]
    fn extracts_provider_and_model() {
        let model = get_custom_llm_provider("anthropic/claude-sonnet-4-5").unwrap();
        assert_eq!(model.provider, LlmProvider::Anthropic);
        assert_eq!(model.model, "claude-sonnet-4-5");
    }
}
