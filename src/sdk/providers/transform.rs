use std::collections::HashMap;

use crate::sdk::codec::WireFormat;

/// Static metadata for a provider id: where to send requests by default, and
/// which wire format the provider speaks (overridable per-deployment via
/// `litellm_params.wire_api`).
#[derive(Debug, Clone)]
pub struct Provider {
    pub default_api_base: String,
    pub default_wire: WireFormat,
}

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Provider>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: &'static str,
        default_api_base: &'static str,
        default_wire: WireFormat,
    ) {
        self.providers.insert(
            id.to_owned(),
            Provider {
                default_api_base: default_api_base.to_owned(),
                default_wire,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<Provider> {
        self.providers.get(id).cloned()
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}
