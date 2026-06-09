use crate::sdk::{codec::WireFormat, providers::transform::ProviderRegistry};

pub fn init(registry: &mut ProviderRegistry) {
    registry.register(
        "anthropic",
        "https://api.anthropic.com",
        WireFormat::AnthropicMessages,
    );
}
