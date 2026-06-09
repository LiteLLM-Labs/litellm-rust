//! Content-block, message, and tool mapping between Anthropic wire JSON and the
//! IR, plus prompt-cache and small shared map helpers.

use serde_json::{json, Map, Value};

use crate::sdk::codec::ir::{
    ContentBlock, ImageSource, Message, ResponseFormat, Role, ToolChoice, ToolDef, Usage,
};

// ---- content block mapping ------------------------------------------------

pub(super) fn block_from_anthropic(v: &Value) -> Option<ContentBlock> {
    let obj = v.as_object()?;
    match obj.get("type").and_then(Value::as_str)? {
        "text" => Some(ContentBlock::Text {
            text: obj.get("text").and_then(Value::as_str)?.to_owned(),
        }),
        "thinking" => Some(ContentBlock::Thinking {
            text: obj
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            signature: obj
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        "tool_use" => Some(ContentBlock::ToolUse {
            id: obj
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            input: obj.get("input").cloned().unwrap_or(Value::Null),
        }),
        "tool_result" => Some(tool_result_from_anthropic(obj)),
        "image" => image_from_anthropic(obj),
        _ => None,
    }
}

fn tool_result_from_anthropic(obj: &Map<String, Value>) -> ContentBlock {
    let content = match obj.get("content") {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(Value::Array(arr)) => arr.iter().filter_map(block_from_anthropic).collect(),
        _ => Vec::new(),
    };
    ContentBlock::ToolResult {
        tool_use_id: obj
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content,
        is_error: obj
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn image_from_anthropic(obj: &Map<String, Value>) -> Option<ContentBlock> {
    let src = obj.get("source")?.as_object()?;
    let source = match src.get("type").and_then(Value::as_str) {
        Some("base64") => ImageSource::Base64 {
            media_type: src
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_owned(),
            data: src
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        Some("url") => ImageSource::Url(src.get("url").and_then(Value::as_str)?.to_owned()),
        _ => return None,
    };
    Some(ContentBlock::Image { source })
}

pub(super) fn block_to_anthropic(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({"type": "text", "text": text}),
        ContentBlock::Thinking { text, signature } => {
            let mut o = json!({"type": "thinking", "thinking": text});
            if let Some(sig) = signature {
                o["signature"] = json!(sig);
            }
            o
        }
        ContentBlock::ToolUse { id, name, input } => {
            json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let rendered = if let [ContentBlock::Text { text }] = content.as_slice() {
                json!(text)
            } else {
                Value::Array(content.iter().map(block_to_anthropic).collect())
            };
            json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": rendered,
                "is_error": is_error,
            })
        }
        ContentBlock::Image { source } => {
            let src = match source {
                ImageSource::Base64 { media_type, data } => {
                    json!({"type": "base64", "media_type": media_type, "data": data})
                }
                ImageSource::Url(url) => json!({"type": "url", "url": url}),
            };
            json!({"type": "image", "source": src})
        }
    }
}

pub(super) fn message_from_anthropic(v: &Value) -> Option<Message> {
    let obj = v.as_object()?;
    let role = match obj.get("role").and_then(Value::as_str)? {
        "assistant" => Role::Assistant,
        _ => Role::User,
    };
    let content = match obj.get("content") {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(Value::Array(arr)) => arr.iter().filter_map(block_from_anthropic).collect(),
        _ => Vec::new(),
    };
    Some(Message { role, content })
}

pub(super) fn tool_from_anthropic(v: &Value) -> Option<ToolDef> {
    let obj = v.as_object()?;
    // Server-side tools carry a versioned `type` (e.g. web_search_20250305);
    // function tools have no `type` or `type:"custom"`.
    if let Some(t) = obj.get("type").and_then(Value::as_str) {
        if t != "custom" {
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
    Some(ToolDef {
        name: obj.get("name").and_then(Value::as_str)?.to_owned(),
        description: obj
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parameters: obj
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"})),
        builtin: None,
    })
}

pub(super) fn response_format_from_anthropic(v: Value) -> Option<ResponseFormat> {
    let obj = v.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
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
            strict: true,
        }),
        Some("json_object") | Some("json") => Some(ResponseFormat::JsonObject),
        _ => None,
    }
}

pub(super) fn tool_to_anthropic(tool: &ToolDef) -> Value {
    let mut o = json!({"name": tool.name, "input_schema": tool.parameters});
    if let Some(desc) = &tool.description {
        o["description"] = json!(desc);
    }
    o
}

pub(super) fn tool_choice_to_anthropic(tc: &ToolChoice) -> Value {
    match tc {
        ToolChoice::Auto => json!({"type": "auto"}),
        ToolChoice::Required => json!({"type": "any"}),
        ToolChoice::None => json!({"type": "none"}),
        ToolChoice::Tool(name) => json!({"type": "tool", "name": name}),
    }
}

pub(super) fn usage_from_anthropic(v: Option<&Value>) -> Usage {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Usage::default();
    };
    let u = |key: &str| obj.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cache_read = u("cache_read_input_tokens");
    let cache_creation = u("cache_creation_input_tokens");
    Usage {
        // Anthropic's `input_tokens` excludes the cached/created portions; add
        // them back so the IR field is the inclusive total (see `Usage` docs).
        input_tokens: u("input_tokens") + cache_read + cache_creation,
        output_tokens: u("output_tokens"),
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
    }
}

// ---- prompt-cache helpers -------------------------------------------------

/// Add a 5-minute `ephemeral` cache breakpoint to a rendered content block.
pub(super) fn set_cache_control(block: &mut Value) {
    if let Some(o) = block.as_object_mut() {
        o.insert("cache_control".to_owned(), json!({"type": "ephemeral"}));
    }
}

/// Whether any element of a raw block/tool array carries a `cache_control` key.
pub(super) fn array_has_cache_control(arr: &[Value]) -> bool {
    arr.iter().any(|b| b.get("cache_control").is_some())
}

/// Whether a raw message has a cache breakpoint, either at the message level or
/// on any of its content blocks.
pub(super) fn message_has_cache_control(msg: &Value) -> bool {
    if msg.get("cache_control").is_some() {
        return true;
    }
    msg.get("content")
        .and_then(Value::as_array)
        .is_some_and(|arr| array_has_cache_control(arr))
}

// ---- shared small helpers -------------------------------------------------

pub(crate) fn take_string(obj: &mut Map<String, Value>, key: &str) -> Option<String> {
    match obj.remove(key) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

pub(crate) fn strip_known(mut obj: Map<String, Value>, known: &[&str]) -> Map<String, Value> {
    for k in known {
        obj.remove(*k);
    }
    obj
}
