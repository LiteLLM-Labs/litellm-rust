use crate::sdk::{codec::WireFormat, providers::transform::ProviderRegistry};

const OPENAI_API_BASE: &str = "https://api.openai.com";

pub fn init(registry: &mut ProviderRegistry) {
    // `openai`/`codex` default to the Responses API (Codex relies on this);
    // `openai_chat` selects Chat Completions.
    registry.register("openai", OPENAI_API_BASE, WireFormat::OpenAiResponses);
    registry.register("codex", OPENAI_API_BASE, WireFormat::OpenAiResponses);
    registry.register("openai_chat", OPENAI_API_BASE, WireFormat::OpenAiChat);
}
