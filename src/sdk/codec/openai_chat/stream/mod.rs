//! Streaming parser (OpenAI SSE -> IR events) and renderer (IR events -> OpenAI SSE).

mod parser;
mod renderer;

pub(super) use parser::OpenAiChatStreamParser;
pub(super) use renderer::OpenAiChatStreamRenderer;
