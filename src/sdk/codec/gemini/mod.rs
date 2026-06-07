//! Gemini (`generateContent` / `streamGenerateContent`) codec.
//!
//! Gemini differs sharply: roles are `user`/`model`, system goes in
//! `systemInstruction`, tool calls/results are `functionCall`/`functionResponse`
//! parts with *object* (not string) arguments, and function calls carry no id —
//! we key tool results back to calls by function name.

mod common;
mod parse;
mod parts;
mod render;
mod stream;
mod stream_render;

use axum::http::{header, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::{
        codec::{
            ir::{ChatRequest, ChatResponse},
            stream::{StreamParser, StreamRenderer},
            ProtocolCodec, RequestCtx,
        },
        router::Deployment,
    },
};

use stream::GeminiStreamParser;
use stream_render::GeminiStreamRenderer;

pub struct GeminiCodec;

impl ProtocolCodec for GeminiCodec {
    fn parse_request(&self, body: Value) -> Result<ChatRequest, GatewayError> {
        parse::parse_request(body)
    }

    fn render_request(&self, req: &ChatRequest) -> Result<Value, GatewayError> {
        render::render_request(req)
    }

    fn parse_response(&self, body: Value) -> Result<ChatResponse, GatewayError> {
        parse::parse_response(body)
    }

    fn render_response(
        &self,
        resp: &ChatResponse,
        ctx: &RequestCtx,
    ) -> Result<Value, GatewayError> {
        render::render_response(resp, ctx)
    }

    fn stream_parser(&self) -> Box<dyn StreamParser> {
        Box::new(GeminiStreamParser::default())
    }

    fn stream_renderer(&self, _ctx: &RequestCtx) -> Box<dyn StreamRenderer> {
        Box::new(GeminiStreamRenderer::default())
    }

    fn outbound_headers(
        &self,
        deployment: &Deployment,
        _inbound: &HeaderMap,
    ) -> Result<HeaderMap, GatewayError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(&deployment.api_key)
                .map_err(|_| GatewayError::InvalidConfig("invalid api_key".to_owned()))?,
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        Ok(headers)
    }

    fn response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let content_type = if stream {
            HeaderValue::from_static("text/event-stream")
        } else {
            upstream
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("application/json"))
        };
        headers.insert(header::CONTENT_TYPE, content_type);
        headers
    }
}
