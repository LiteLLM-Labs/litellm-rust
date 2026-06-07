//! Anthropic Messages (`/v1/messages`) codec. Closest to the IR shape since the
//! IR mirrors Anthropic content blocks.

mod blocks;
mod parse;
mod render;
mod stream;

pub(crate) use blocks::{strip_known, take_string};

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

use stream::{AnthropicStreamParser, AnthropicStreamRenderer};

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u64 = 4096;

const KNOWN_REQUEST_KEYS: &[&str] = &[
    "model",
    "system",
    "messages",
    "tools",
    "tool_choice",
    "thinking",
    "output_config",
    "output_format",
    "max_tokens",
    "temperature",
    "top_p",
    "stop_sequences",
    "stream",
];

pub struct AnthropicCodec;

impl ProtocolCodec for AnthropicCodec {
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
        Box::new(AnthropicStreamParser::default())
    }

    fn stream_renderer(&self, ctx: &RequestCtx) -> Box<dyn StreamRenderer> {
        Box::new(AnthropicStreamRenderer {
            model: ctx.model.clone(),
        })
    }

    fn outbound_headers(
        &self,
        deployment: &Deployment,
        inbound: &HeaderMap,
    ) -> Result<HeaderMap, GatewayError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&deployment.api_key)
                .map_err(|_| GatewayError::InvalidConfig("invalid api_key".to_owned()))?,
        );
        headers.insert(
            "anthropic-version",
            inbound
                .get("anthropic-version")
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION)),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(beta) = inbound.get("anthropic-beta") {
            headers.insert("anthropic-beta", beta.clone());
        }
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
        if let Some(request_id) = upstream.get("request-id").cloned() {
            headers.insert("request-id", request_id);
        }
        headers
    }
}

#[cfg(test)]
mod cache_tests {
    use super::blocks::usage_from_anthropic;
    use super::stream::AnthropicStreamParser;
    use super::*;
    use crate::sdk::codec::ir::{ChatResponse, StreamEvent};

    #[test]
    fn parses_and_renders_cache_control_breakpoints() {
        let body = serde_json::json!({
            "model": "claude",
            "system": [
                {"type": "text", "text": "you are helpful"},
                {"type": "text", "text": "rules", "cache_control": {"type": "ephemeral"}}
            ],
            "tools": [
                {"name": "a", "input_schema": {"type": "object"}},
                {"name": "b", "input_schema": {"type": "object"},
                 "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}
                ]}
            ],
            "max_tokens": 100
        });
        let req = AnthropicCodec.parse_request(body).unwrap();
        assert!(req.cache.tools);
        assert!(req.cache.system);
        assert_eq!(req.cache.messages, vec![0]);

        let out = AnthropicCodec.render_request(&req).unwrap();
        let sys = out["system"].as_array().unwrap();
        assert!(sys[0].get("cache_control").is_none());
        assert!(sys[1].get("cache_control").is_some());
        let tools = out["tools"].as_array().unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert!(tools[1].get("cache_control").is_some());
        let content = out["messages"][0]["content"].as_array().unwrap();
        assert!(content.last().unwrap().get("cache_control").is_some());
    }

    #[test]
    fn no_breakpoints_when_client_sets_none() {
        let body = serde_json::json!({
            "model": "claude",
            "system": [{"type": "text", "text": "hi"}],
            "messages": [{"role": "user", "content": "hello"}],
            "max_tokens": 100
        });
        let req = AnthropicCodec.parse_request(body).unwrap();
        assert!(req.cache.is_empty());
        let out = AnthropicCodec.render_request(&req).unwrap();
        assert!(out["system"][0].get("cache_control").is_none());
    }

    #[test]
    fn stream_parser_folds_cache_usage_from_message_start() {
        use crate::sdk::codec::stream::SseEvent;
        let mut p = AnthropicStreamParser::default();
        p.push(&SseEvent {
            event: Some("message_start".to_owned()),
            data: r#"{"type":"message_start","message":{"id":"m","model":"claude","usage":{"input_tokens":5,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200,"output_tokens":0}}}"#.to_owned(),
        })
        .unwrap();
        let evs = p
            .push(&SseEvent {
                event: Some("message_delta".to_owned()),
                data: r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#.to_owned(),
            })
            .unwrap();
        match &evs[0] {
            StreamEvent::MessageDelta { usage: Some(u), .. } => {
                assert_eq!(u.input_tokens, 1205);
                assert_eq!(u.output_tokens, 42);
                assert_eq!(u.cache_read_input_tokens, 1000);
                assert_eq!(u.cache_creation_input_tokens, 200);
            }
            other => panic!("expected MessageDelta with usage, got {other:?}"),
        }
    }

    #[test]
    fn usage_makes_input_inclusive_and_round_trips() {
        let usage = usage_from_anthropic(Some(&serde_json::json!({
            "input_tokens": 50,
            "output_tokens": 10,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 1000
        })));
        assert_eq!(usage.input_tokens, 1250);
        assert_eq!(usage.cache_read_input_tokens, 1000);
        assert_eq!(usage.cache_creation_input_tokens, 200);
        assert_eq!(usage.non_cached_input_tokens(), 50);

        let resp = ChatResponse {
            usage,
            ..Default::default()
        };
        let ctx = RequestCtx {
            model: "claude".to_owned(),
            stream: false,
        };
        let out = AnthropicCodec.render_response(&resp, &ctx).unwrap();
        assert_eq!(out["usage"]["input_tokens"], 50);
        assert_eq!(out["usage"]["cache_read_input_tokens"], 1000);
        assert_eq!(out["usage"]["cache_creation_input_tokens"], 200);
    }
}
