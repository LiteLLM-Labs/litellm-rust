use crate::sdk::{codec::WireFormat, providers::transform::ProviderRegistry};

pub fn init(registry: &mut ProviderRegistry) {
    registry.register(
        "gemini",
        "https://generativelanguage.googleapis.com",
        WireFormat::Gemini,
    );
}
