//! Request/response parsing helpers: OpenAI wire shapes into IR.

use serde_json::{json, Map, Value};

use crate::errors::GatewayError;
use crate::sdk::codec::ir::{
    ChatResponse, ContentBlock, Effort, ImageSource, Message, ReasoningConfig, ResponseFormat,
    Role, StopReason, ToolChoice, ToolDef, Usage,
};

pub(super) fn parse_response(body: Value) -> Result<ChatResponse, GatewayError> {
    let obj = body.as_object().ok_or_else(|| {
        GatewayError::InvalidJsonMessage("response body must be a JSON object".to_owned())
    })?;
    let choice = obj
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first());
    let message = choice.and_then(|c| c.get("message"));

    let mut content = Vec::new();
    // Some OpenAI-compatible upstreams return reasoning in `reasoning_content`;
    // keep it so cross-protocol clients don't lose the thinking block.
    if let Some(reasoning) = message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        content.push(ContentBlock::Thinking {
            text: reasoning.to_owned(),
            signature: None,
        });
    }
    if let Some(text) = message
        .and_then(|m| m.get("content").or_else(|| m.get("refusal")))
        .and_then(Value::as_str)
    {
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_owned(),
            });
        }
    }
    if let Some(tcs) = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for tc in tcs {
            if let Some(block) = tool_call_to_block(tc) {
                content.push(block);
            }
        }
    }

    Ok(ChatResponse {
        id: obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        model: obj
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content,
        stop_reason: choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(Value::as_str)
            .map(StopReason::from_openai),
        usage: usage_from_openai(obj.get("usage")),
    })
}

pub(super) fn content_to_text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(arr)) => {
            let mut text = String::new();
            for part in arr {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some(text)
        }
        _ => None,
    }
}

pub(super) fn parse_user(mo: &Map<String, Value>) -> Message {
    let content = match mo.get("content") {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(Value::Array(arr)) => arr.iter().filter_map(part_to_block).collect(),
        _ => Vec::new(),
    };
    Message {
        role: Role::User,
        content,
    }
}

pub(super) fn parse_assistant(mo: &Map<String, Value>) -> Message {
    let mut content = Vec::new();
    if let Some(text) = mo.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_owned(),
            });
        }
    }
    if let Some(tcs) = mo.get("tool_calls").and_then(Value::as_array) {
        for tc in tcs {
            if let Some(block) = tool_call_to_block(tc) {
                content.push(block);
            }
        }
    }
    Message {
        role: Role::Assistant,
        content,
    }
}

pub(super) fn parse_tool_message(mo: &Map<String, Value>) -> Message {
    let tool_use_id = mo
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let text = content_to_text(mo.get("content")).unwrap_or_default();
    Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            tool_use_id,
            content: vec![ContentBlock::Text { text }],
            is_error: false,
        }],
    }
}

fn part_to_block(part: &Value) -> Option<ContentBlock> {
    let obj = part.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("text") => Some(ContentBlock::Text {
            text: obj.get("text").and_then(Value::as_str)?.to_owned(),
        }),
        Some("image_url") => {
            let url = obj
                .get("image_url")
                .and_then(|iu| iu.get("url"))
                .and_then(Value::as_str)?;
            Some(ContentBlock::Image {
                source: data_url_to_source(url),
            })
        }
        _ => None,
    }
}

pub(crate) fn data_url_to_source(url: &str) -> ImageSource {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(",") {
            let media_type = meta.split(';').next().unwrap_or("image/png").to_owned();
            return ImageSource::Base64 {
                media_type,
                data: data.to_owned(),
            };
        }
    }
    ImageSource::Url(url.to_owned())
}

pub(super) fn tool_call_to_block(tc: &Value) -> Option<ContentBlock> {
    let func = tc.get("function")?;
    let args = func
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let input = serde_json::from_str(args).unwrap_or_else(|_| json!(args));
    Some(ContentBlock::ToolUse {
        id: tc
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name: func
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input,
    })
}

pub(super) fn tool_from_openai(v: &Value) -> Option<ToolDef> {
    let func = v.get("function")?;
    Some(ToolDef {
        name: func.get("name").and_then(Value::as_str)?.to_owned(),
        description: func
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parameters: func
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"})),
        builtin: None,
    })
}

pub(super) fn response_format_from_openai(v: Value) -> Option<ResponseFormat> {
    let obj = v.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("json_object") => Some(ResponseFormat::JsonObject),
        Some("json_schema") => {
            let js = obj.get("json_schema")?;
            Some(ResponseFormat::JsonSchema {
                name: js
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("response")
                    .to_owned(),
                schema: js
                    .get("schema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"})),
                strict: js.get("strict").and_then(Value::as_bool).unwrap_or(true),
            })
        }
        _ => None,
    }
}

pub(super) fn parse_tool_choice(v: Value) -> Option<ToolChoice> {
    match v {
        Value::String(s) => match s.as_str() {
            "auto" => Some(ToolChoice::Auto),
            "none" => Some(ToolChoice::None),
            "required" => Some(ToolChoice::Required),
            _ => None,
        },
        Value::Object(o) => o
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map(|n| ToolChoice::Tool(n.to_owned())),
        _ => None,
    }
}

pub(super) fn usage_from_openai(v: Option<&Value>) -> Usage {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Usage::default();
    };
    // OpenAI's `prompt_tokens` is already inclusive of cached tokens.
    let cached = obj
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        input_tokens: obj
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: obj
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}

pub(super) fn effort_to_reasoning(v: Value) -> Option<ReasoningConfig> {
    v.as_str().and_then(Effort::parse).map(|e| ReasoningConfig {
        effort: Some(e),
        budget_tokens: None,
    })
}
