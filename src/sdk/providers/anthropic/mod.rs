pub mod transformation;

use crate::sdk::providers::transform::ProviderRegistry;
use transformation::AnthropicTransformation;

pub fn init(registry: &mut ProviderRegistry) {
    registry.register("anthropic", "https://api.anthropic.com", AnthropicTransformation);
}
