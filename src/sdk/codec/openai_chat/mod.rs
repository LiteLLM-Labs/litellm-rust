//! OpenAI Chat Completions (`/v1/chat/completions`) codec. Tool calls live in
//! `tool_calls` / `role:"tool"` messages with JSON-string arguments, so this is
//! the most structurally distant from the IR.

mod parse;
mod render;
mod stream;

use std::collections::HashMap;

use axum::http::{header, HeaderMap, HeaderValue};
use serde_json::{Map, Value};

use crate::{
    errors::GatewayError,
    sdk::{
        codec::{
            anthropic::{strip_known, take_string},
            ir::{CacheMarkers, ChatRequest, ChatResponse, ContentBlock, Message},
            stream::{StreamParser, StreamRenderer},
            ProtocolCodec, RequestCtx,
        },
        router::Deployment,
    },
};

pub(crate) use render::{join_text, source_to_data_url, value_to_args};

use parse::{
    content_to_text, effort_to_reasoning, parse_assistant, parse_tool_choice, parse_tool_message,
    parse_user, response_format_from_openai, tool_from_openai,
};
use stream::{OpenAiChatStreamParser, OpenAiChatStreamRenderer};

const KNOWN_REQUEST_KEYS: &[&str] = &[
    "model",
    "messages",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "response_format",
    "reasoning_effort",
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "stop",
    "stream",
    "stream_options",
];

pub struct OpenAiChatCodec;

/// Hoist system/developer messages into IR `system`; everything else maps to IR
/// messages.
fn hoist_messages(obj: &mut Map<String, Value>) -> (Vec<ContentBlock>, Vec<Message>) {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    if let Some(Value::Array(arr)) = obj.remove("messages") {
        for m in &arr {
            let Some(mo) = m.as_object() else { continue };
            let role = mo.get("role").and_then(Value::as_str).unwrap_or("user");
            match role {
                "system" | "developer" => {
                    if let Some(text) = content_to_text(mo.get("content")) {
                        system.push(ContentBlock::Text { text });
                    }
                }
                "assistant" => messages.push(parse_assistant(mo)),
                "tool" => messages.push(parse_tool_message(mo)),
                _ => messages.push(parse_user(mo)),
            }
        }
    }
    (system, messages)
}

fn parse_stop(obj: &mut Map<String, Value>) -> Vec<String> {
    match obj.remove("stop") {
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

impl ProtocolCodec for OpenAiChatCodec {
    fn parse_request(&self, body: Value) -> Result<ChatRequest, GatewayError> {
        let Value::Object(mut obj) = body else {
            return Err(GatewayError::InvalidJsonMessage(
                "request body must be a JSON object".to_owned(),
            ));
        };

        let model = take_string(&mut obj, "model").unwrap_or_default();
        let (system, messages) = hoist_messages(&mut obj);
        let tools = match obj.remove("tools") {
            Some(Value::Array(arr)) => arr.iter().filter_map(tool_from_openai).collect(),
            _ => Vec::new(),
        };
        let tool_choice = obj.remove("tool_choice").and_then(parse_tool_choice);
        let stop = parse_stop(&mut obj);
        let max_tokens = obj
            .remove("max_tokens")
            .or_else(|| obj.remove("max_completion_tokens"))
            .and_then(|v| v.as_u64());

        let req = ChatRequest {
            model,
            system,
            messages,
            tools,
            // OpenAI prefix caching is automatic; nothing to carry from the wire.
            cache: CacheMarkers::default(),
            tool_choice,
            parallel_tool_calls: obj.remove("parallel_tool_calls").and_then(|v| v.as_bool()),
            response_format: obj
                .remove("response_format")
                .and_then(response_format_from_openai),
            reasoning: obj.remove("reasoning_effort").and_then(effort_to_reasoning),
            max_tokens,
            temperature: obj.remove("temperature").and_then(|v| v.as_f64()),
            top_p: obj.remove("top_p").and_then(|v| v.as_f64()),
            stop,
            stream: obj
                .remove("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            extra: strip_known(obj, KNOWN_REQUEST_KEYS),
        };
        Ok(req)
    }

    fn render_request(&self, req: &ChatRequest) -> Result<Value, GatewayError> {
        Ok(render::render_request(req))
    }

    fn parse_response(&self, body: Value) -> Result<ChatResponse, GatewayError> {
        parse::parse_response(body)
    }

    fn render_response(
        &self,
        resp: &ChatResponse,
        ctx: &RequestCtx,
    ) -> Result<Value, GatewayError> {
        Ok(render::render_response(resp, &ctx.model))
    }

    fn stream_parser(&self) -> Box<dyn StreamParser> {
        Box::new(OpenAiChatStreamParser::default())
    }

    fn stream_renderer(&self, ctx: &RequestCtx) -> Box<dyn StreamRenderer> {
        Box::new(OpenAiChatStreamRenderer {
            model: ctx.model.clone(),
            id: String::new(),
            role_sent: false,
            tool_index: HashMap::new(),
            next_tool: 0,
            done_sent: false,
        })
    }

    fn outbound_headers(
        &self,
        deployment: &Deployment,
        _inbound: &HeaderMap,
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
        Ok(headers)
    }

    fn response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap {
        openai_response_headers(upstream, stream)
    }
}

pub(crate) fn openai_response_headers(upstream: &HeaderMap, stream: bool) -> HeaderMap {
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
    if let Some(request_id) = upstream.get("x-request-id").cloned() {
        headers.insert("x-request-id", request_id);
    }
    headers
}
