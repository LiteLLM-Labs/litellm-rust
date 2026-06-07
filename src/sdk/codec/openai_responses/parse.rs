//! Request/response parsing for the Responses codec.

use serde_json::{json, Value};

use crate::{
    errors::GatewayError,
    sdk::codec::{
        anthropic::{strip_known, take_string},
        ir::{
            CacheMarkers, ChatRequest, ContentBlock, Effort, ImageSource, Message, ReasoningConfig,
            ResponseFormat, Role, ToolChoice, ToolDef, Usage,
        },
    },
};

const KNOWN_REQUEST_KEYS: &[&str] = &[
    "model",
    "instructions",
    "input",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "text",
    "reasoning",
    "max_output_tokens",
    "temperature",
    "top_p",
    "stream",
];

pub(super) fn parse_request(body: Value) -> Result<ChatRequest, GatewayError> {
    let Value::Object(mut obj) = body else {
        return Err(GatewayError::InvalidJsonMessage(
            "request body must be a JSON object".to_owned(),
        ));
    };

    let model = take_string(&mut obj, "model").unwrap_or_default();
    let system = match take_string(&mut obj, "instructions") {
        Some(s) => vec![ContentBlock::Text { text: s }],
        None => Vec::new(),
    };

    let messages = match obj.remove("input") {
        Some(Value::String(s)) => vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: s }],
        }],
        Some(Value::Array(arr)) => parse_input_items(&arr),
        _ => Vec::new(),
    };

    let tools = match obj.remove("tools") {
        Some(Value::Array(arr)) => arr.iter().filter_map(tool_from_responses).collect(),
        _ => Vec::new(),
    };

    let response_format = obj
        .remove("text")
        .and_then(|t| t.get("format").cloned())
        .and_then(response_format_from_responses);

    let req = ChatRequest {
        model,
        system,
        messages,
        tools,
        // Responses prefix caching is automatic; nothing to carry from the wire.
        cache: CacheMarkers::default(),
        tool_choice: obj.remove("tool_choice").and_then(parse_tool_choice),
        parallel_tool_calls: obj.remove("parallel_tool_calls").and_then(|v| v.as_bool()),
        response_format,
        reasoning: obj.remove("reasoning").and_then(parse_reasoning),
        max_tokens: obj.remove("max_output_tokens").and_then(|v| v.as_u64()),
        temperature: obj.remove("temperature").and_then(|v| v.as_f64()),
        top_p: obj.remove("top_p").and_then(|v| v.as_f64()),
        stop: Vec::new(),
        stream: obj
            .remove("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        extra: strip_known(obj, KNOWN_REQUEST_KEYS),
    };
    Ok(req)
}

fn parse_reasoning(r: Value) -> Option<ReasoningConfig> {
    r.get("effort")
        .and_then(Value::as_str)
        .and_then(Effort::parse)
        .map(|e| ReasoningConfig {
            effort: Some(e),
            budget_tokens: None,
        })
}

// ---- request item mapping -------------------------------------------------

fn parse_input_items(arr: &[Value]) -> Vec<Message> {
    let mut messages = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        match obj.get("type").and_then(Value::as_str) {
            Some("function_call") => messages.push(Message {
                role: Role::Assistant,
                content: vec![function_call_to_block(item)],
            }),
            Some("function_call_output") => {
                let call_id = obj
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let output = match obj.get("output") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                messages.push(Message {
                    role: Role::Tool,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: call_id,
                        content: vec![ContentBlock::Text { text: output }],
                        is_error: false,
                    }],
                });
            }
            // A message item (explicit "message" type or a bare {role, content}).
            _ => {
                let role = match obj.get("role").and_then(Value::as_str) {
                    Some("assistant") => Role::Assistant,
                    Some("system") | Some("developer") => Role::System,
                    _ => Role::User,
                };
                let content = match obj.get("content") {
                    Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
                    Some(Value::Array(parts)) => {
                        parts.iter().filter_map(content_part_to_block).collect()
                    }
                    _ => Vec::new(),
                };
                messages.push(Message { role, content });
            }
        }
    }
    messages
}

fn content_part_to_block(part: &Value) -> Option<ContentBlock> {
    let obj = part.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") | Some("text") => Some(ContentBlock::Text {
            text: obj.get("text").and_then(Value::as_str)?.to_owned(),
        }),
        // `input_image.image_url`: a string (URL/data: URL) or Chat-style `{url}`.
        Some("input_image") => {
            let image_url = obj.get("image_url")?;
            let url = image_url
                .as_str()
                .or_else(|| image_url.get("url").and_then(Value::as_str))?;
            Some(ContentBlock::Image {
                source: data_url_to_source(url),
            })
        }
        _ => None,
    }
}

fn data_url_to_source(url: &str) -> ImageSource {
    if let Some((meta, data)) = url.strip_prefix("data:").and_then(|r| r.split_once(',')) {
        let media_type = meta.split(';').next().unwrap_or("image/png").to_owned();
        return ImageSource::Base64 {
            media_type,
            data: data.to_owned(),
        };
    }
    ImageSource::Url(url.to_owned())
}

pub(super) fn function_call_to_block(item: &Value) -> ContentBlock {
    let args = item
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let input = serde_json::from_str(args).unwrap_or_else(|_| json!(args));
    ContentBlock::ToolUse {
        id: item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name: item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input,
    }
}

fn tool_from_responses(v: &Value) -> Option<ToolDef> {
    let obj = v.as_object()?;
    // Built-in tools (web_search, file_search, code_interpreter, image_generation,
    // computer_use, mcp, …) carry a non-"function" type.
    if let Some(t) = obj.get("type").and_then(Value::as_str) {
        if t != "function" {
            return Some(ToolDef {
                name: obj
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(t)
                    .to_owned(),
                description: None,
                parameters: json!({"type": "object"}),
                builtin: Some(v.clone()),
            });
        }
    }
    // Function tool — flat (name at top level), but tolerate the nested Chat shape.
    let name = obj
        .get("name")
        .or_else(|| obj.get("function").and_then(|f| f.get("name")))
        .and_then(Value::as_str)?;
    let description = obj
        .get("description")
        .or_else(|| obj.get("function").and_then(|f| f.get("description")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let parameters = obj
        .get("parameters")
        .or_else(|| obj.get("function").and_then(|f| f.get("parameters")))
        .cloned()
        .unwrap_or_else(|| json!({"type": "object"}));
    Some(ToolDef {
        name: name.to_owned(),
        description,
        parameters,
        builtin: None,
    })
}

fn response_format_from_responses(v: Value) -> Option<ResponseFormat> {
    let obj = v.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("json_object") => Some(ResponseFormat::JsonObject),
        Some("json_schema") => Some(ResponseFormat::JsonSchema {
            name: obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("response")
                .to_owned(),
            schema: obj
                .get("schema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
            strict: obj.get("strict").and_then(Value::as_bool).unwrap_or(true),
        }),
        _ => None,
    }
}

fn parse_tool_choice(v: Value) -> Option<ToolChoice> {
    match v {
        Value::String(s) => match s.as_str() {
            "auto" => Some(ToolChoice::Auto),
            "none" => Some(ToolChoice::None),
            "required" => Some(ToolChoice::Required),
            _ => None,
        },
        Value::Object(o) => o
            .get("name")
            .and_then(Value::as_str)
            .map(|n| ToolChoice::Tool(n.to_owned())),
        _ => None,
    }
}

pub(super) fn usage_from_responses(v: Option<&Value>) -> Usage {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Usage::default();
    };
    // Responses `input_tokens` is already inclusive of cached tokens.
    let cached = obj
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        input_tokens: obj.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: obj
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}
