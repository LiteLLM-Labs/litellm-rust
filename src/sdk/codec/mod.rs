//! Protocol codecs and the canonical IR pivot.
//!
//! Each wire protocol (Anthropic Messages, OpenAI Chat Completions, OpenAI
//! Responses, Gemini generateContent) implements [`ProtocolCodec`]: it parses
//! its wire shape into the IR and renders the IR back out. A request is
//! translated by parsing with the inbound codec and rendering with the outbound
//! codec, so N protocols need N codecs rather than N×N translators.

pub mod anthropic;
pub mod cache_inject;
pub mod gemini;
pub mod ir;
pub mod openai_chat;
pub mod openai_responses;
pub mod stream;

use axum::http::HeaderMap;
use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::{
        codec::{
            ir::{ChatRequest, ChatResponse},
            stream::{StreamParser, StreamRenderer},
        },
        router::Deployment,
    },
};

/// The four wire formats the gateway can speak, inbound and outbound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    AnthropicMessages,
    OpenAiChat,
    OpenAiResponses,
    Gemini,
}

impl WireFormat {
    /// Parse a `wire_api` config override value.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "messages" | "anthropic_messages" => Some(Self::AnthropicMessages),
            "chat" | "openai_chat" | "chat_completions" => Some(Self::OpenAiChat),
            "responses" | "openai_responses" => Some(Self::OpenAiResponses),
            "gemini" | "google" => Some(Self::Gemini),
            _ => None,
        }
    }
}

/// Context from the inbound request that codecs need when rendering outbound
/// requests or inbound-shaped responses.
#[derive(Debug, Clone)]
pub struct RequestCtx {
    /// Public model name the client asked for; echoed back in responses.
    pub model: String,
    pub stream: bool,
}

/// Translates between one wire protocol and the canonical IR, both directions.
pub trait ProtocolCodec: Send + Sync {
    /// Inbound: client request body (this protocol) → IR. The pipeline sets the
    /// final `model`, so implementations may leave `ChatRequest::model` empty.
    fn parse_request(&self, body: Value) -> Result<ChatRequest, GatewayError>;

    /// Outbound: IR → provider request body (this protocol).
    fn render_request(&self, req: &ChatRequest) -> Result<Value, GatewayError>;

    /// Outbound: provider response body (this protocol) → IR.
    fn parse_response(&self, body: Value) -> Result<ChatResponse, GatewayError>;

    /// Inbound: IR → client response body (this protocol).
    fn render_response(&self, resp: &ChatResponse, ctx: &RequestCtx)
        -> Result<Value, GatewayError>;

    /// Outbound: stateful parser for the provider's SSE stream → IR events.
    fn stream_parser(&self) -> Box<dyn StreamParser>;

    /// Inbound: stateful renderer for IR events → client SSE bytes.
    fn stream_renderer(&self, ctx: &RequestCtx) -> Box<dyn StreamRenderer>;

    /// Build outbound auth/headers for this protocol from the resolved
    /// deployment and the inbound request headers.
    fn outbound_headers(
        &self,
        deployment: &Deployment,
        inbound: &HeaderMap,
    ) -> Result<HeaderMap, GatewayError>;

    /// Headers to return to the client for this protocol.
    fn response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap;

    /// Inbound header names this codec forwards upstream that change the response,
    /// and so must be part of the exact cache key — otherwise two requests that
    /// differ only by such a header would collide and replay each other's answer.
    /// Deliberately excludes volatile per-request/session identifiers (request ids,
    /// session/thread/turn metadata): those vary on every call and would make the
    /// cache effectively never hit. Empty by default; override per codec so the key
    /// stays aligned with what `outbound_headers` actually forwards.
    fn cache_key_headers(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Resolve a codec for a wire format. Codecs are zero-sized unit structs, so the
/// references promote to `'static`.
pub fn codec_for(wire: WireFormat) -> &'static dyn ProtocolCodec {
    match wire {
        WireFormat::AnthropicMessages => &anthropic::AnthropicCodec,
        WireFormat::OpenAiChat => &openai_chat::OpenAiChatCodec,
        WireFormat::OpenAiResponses => &openai_responses::OpenAiResponsesCodec,
        WireFormat::Gemini => &gemini::GeminiCodec,
    }
}
