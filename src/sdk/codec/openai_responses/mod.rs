//! OpenAI Responses (`/v1/responses`) codec.
//!
//! The Responses API differs from Chat Completions: system → `instructions`,
//! `messages` → an `input` array of items, tool calls/results are top-level
//! `function_call` / `function_call_output` items, and tools are flat.

mod decode;
mod parse;
mod render;
mod render_stream;
mod stream;

use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::{
        codec::{
            ir::{ChatRequest, ChatResponse},
            openai_chat::openai_response_headers,
            stream::{StreamParser, StreamRenderer},
            ProtocolCodec, RequestCtx,
        },
        router::Deployment,
    },
};

use render_stream::ResponsesStreamRenderer;
use stream::ResponsesStreamParser;

const FORWARDED_HEADERS: &[&str] = &[
    "accept",
    "originator",
    "session-id",
    "thread-id",
    "x-client-request-id",
    "x-codex-beta-features",
    "x-codex-turn-metadata",
    "x-codex-window-id",
];

pub struct OpenAiResponsesCodec;

impl ProtocolCodec for OpenAiResponsesCodec {
    fn parse_request(&self, body: Value) -> Result<ChatRequest, GatewayError> {
        parse::parse_request(body)
    }

    fn render_request(&self, req: &ChatRequest) -> Result<Value, GatewayError> {
        render::render_request(req)
    }

    fn parse_response(&self, body: Value) -> Result<ChatResponse, GatewayError> {
        decode::parse_response(body)
    }

    fn render_response(
        &self,
        resp: &ChatResponse,
        ctx: &RequestCtx,
    ) -> Result<Value, GatewayError> {
        render::render_response(resp, ctx)
    }

    fn stream_parser(&self) -> Box<dyn StreamParser> {
        Box::new(ResponsesStreamParser::default())
    }

    fn stream_renderer(&self, ctx: &RequestCtx) -> Box<dyn StreamRenderer> {
        Box::new(ResponsesStreamRenderer {
            model: ctx.model.clone(),
            id: String::new(),
            next_oi: 0,
            stop_reason: None,
            usage: None,
        })
    }

    fn outbound_headers(
        &self,
        deployment: &Deployment,
        inbound: &HeaderMap,
    ) -> Result<HeaderMap, GatewayError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", deployment.api_key))
                .map_err(|_| GatewayError::InvalidConfig("invalid api_key".to_owned()))?,
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for name in FORWARDED_HEADERS {
            if let Some(value) = inbound.get(*name) {
                if let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) {
                    headers.insert(header_name, value.clone());
                }
            }
        }
        Ok(headers)
    }

    fn response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap {
        openai_response_headers(upstream, stream)
    }

    fn cache_key_headers(&self) -> &'static [&'static str] {
        // Of the forwarded headers, only the beta-feature toggle shapes the answer;
        // the rest (session/thread/turn/window/request ids) are volatile telemetry.
        &["x-codex-beta-features"]
    }
}
