//! Anthropic request/response parsing into the IR.

use serde_json::{Map, Value};

use crate::{
    errors::GatewayError,
    sdk::codec::ir::{
        CacheMarkers, ChatRequest, ChatResponse, ContentBlock, Message, ReasoningConfig,
        StopReason, ToolChoice, ToolDef,
    },
};

use super::blocks::{
    array_has_cache_control, block_from_anthropic, message_from_anthropic,
    message_has_cache_control, response_format_from_anthropic, strip_known, take_string,
    tool_from_anthropic, usage_from_anthropic,
};
use super::KNOWN_REQUEST_KEYS;

pub(super) fn parse_request(body: Value) -> Result<ChatRequest, GatewayError> {
    let Value::Object(mut obj) = body else {
        return Err(GatewayError::InvalidJsonMessage(
            "request body must be a JSON object".to_owned(),
        ));
    };

    let model = take_string(&mut obj, "model").unwrap_or_default();
    let (system, system_cached) = parse_system(obj.remove("system"));
    let (messages, message_cache_idx) = parse_messages(obj.remove("messages"));
    let (tools, tools_cached) = parse_tools(obj.remove("tools"));
    let (tool_choice, parallel_tool_calls) = parse_tool_choice(obj.remove("tool_choice"));

    let reasoning = parse_reasoning(obj.remove("thinking"));
    let response_format = obj
        .remove("output_format")
        .or_else(|| {
            obj.remove("output_config")
                .and_then(|oc| oc.get("format").cloned())
        })
        .and_then(response_format_from_anthropic);
    let stop = parse_stop(obj.remove("stop_sequences"));

    Ok(ChatRequest {
        model,
        system,
        messages,
        tools,
        cache: CacheMarkers {
            tools: tools_cached,
            system: system_cached,
            messages: message_cache_idx,
        },
        tool_choice,
        parallel_tool_calls,
        response_format,
        reasoning,
        max_tokens: obj.remove("max_tokens").and_then(|v| v.as_u64()),
        temperature: obj.remove("temperature").and_then(|v| v.as_f64()),
        top_p: obj.remove("top_p").and_then(|v| v.as_f64()),
        stop,
        stream: obj
            .remove("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        extra: strip_known(obj, KNOWN_REQUEST_KEYS),
    })
}

fn parse_system(raw: Option<Value>) -> (Vec<ContentBlock>, bool) {
    let cached = matches!(&raw, Some(Value::Array(arr)) if array_has_cache_control(arr));
    let system = match raw {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s }],
        Some(Value::Array(arr)) => arr.iter().filter_map(block_from_anthropic).collect(),
        _ => Vec::new(),
    };
    (system, cached)
}

fn parse_messages(raw: Option<Value>) -> (Vec<Message>, Vec<usize>) {
    let mut cache_idx = Vec::new();
    let messages = match raw {
        Some(Value::Array(arr)) => {
            let mut out = Vec::new();
            for raw in &arr {
                if let Some(msg) = message_from_anthropic(raw) {
                    if message_has_cache_control(raw) {
                        cache_idx.push(out.len());
                    }
                    out.push(msg);
                }
            }
            out
        }
        _ => Vec::new(),
    };
    (messages, cache_idx)
}

fn parse_tools(raw: Option<Value>) -> (Vec<ToolDef>, bool) {
    let cached = matches!(&raw, Some(Value::Array(arr)) if array_has_cache_control(arr));
    let tools = match raw {
        Some(Value::Array(arr)) => arr.iter().filter_map(tool_from_anthropic).collect(),
        _ => Vec::new(),
    };
    (tools, cached)
}

fn parse_tool_choice(raw: Option<Value>) -> (Option<ToolChoice>, Option<bool>) {
    let parallel_tool_calls = raw
        .as_ref()
        .and_then(|tc| tc.get("disable_parallel_tool_use"))
        .and_then(Value::as_bool)
        .map(|disabled| !disabled);
    let tool_choice = raw.and_then(|tc| match tc {
        Value::Object(o) => match o.get("type").and_then(Value::as_str) {
            Some("auto") => Some(ToolChoice::Auto),
            Some("any") => Some(ToolChoice::Required),
            Some("none") => Some(ToolChoice::None),
            Some("tool") => o
                .get("name")
                .and_then(Value::as_str)
                .map(|n| ToolChoice::Tool(n.to_owned())),
            _ => None,
        },
        _ => None,
    });
    (tool_choice, parallel_tool_calls)
}

fn parse_reasoning(raw: Option<Value>) -> Option<ReasoningConfig> {
    raw.and_then(|t| {
        let enabled = t.get("type").and_then(Value::as_str) == Some("enabled");
        let budget = t.get("budget_tokens").and_then(Value::as_u64);
        (enabled || budget.is_some()).then_some(ReasoningConfig {
            effort: None,
            budget_tokens: budget,
        })
    })
}

fn parse_stop(raw: Option<Value>) -> Vec<String> {
    match raw {
        Some(Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn parse_response(body: Value) -> Result<ChatResponse, GatewayError> {
    let obj = body.as_object().ok_or_else(|| {
        GatewayError::InvalidJsonMessage("response body must be a JSON object".to_owned())
    })?;
    let content = obj
        .get("content")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(block_from_anthropic).collect())
        .unwrap_or_default();
    Ok(ChatResponse {
        id: str_field(obj, "id"),
        model: str_field(obj, "model"),
        content,
        stop_reason: obj
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(StopReason::from_anthropic),
        usage: usage_from_anthropic(obj.get("usage")),
    })
}

fn str_field(obj: &Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
