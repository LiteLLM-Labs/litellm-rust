use crate::{app::errors::GatewayError, config::schema::GatewayConfig};

pub fn validate_config(config: &GatewayConfig) -> Result<(), GatewayError> {
    if config.model_list.is_empty() {
        return Err(GatewayError::InvalidConfig(
            "model_list must contain at least one model".to_owned(),
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

    Ok(())
}
