//! Rendering helpers: IR into OpenAI wire shapes.

use serde_json::{json, Map, Value};

use crate::sdk::codec::ir::{
    ChatRequest, ChatResponse, ContentBlock, ImageSource, Message, ResponseFormat, Role,
    StopReason, ToolChoice, ToolDef, Usage,
};

pub(super) fn render_request(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if !req.system.is_empty() {
        messages.push(json!({"role": "system", "content": join_text(&req.system)}));
    }
    for msg in &req.messages {
        flatten_message(msg, &mut messages);
    }

    let mut obj = Map::new();
    obj.insert("model".to_owned(), json!(req.model));
    obj.insert("messages".to_owned(), Value::Array(messages));
    let function_tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| t.builtin.is_none())
        .map(tool_to_openai)
        .collect();
    let has_tools = !function_tools.is_empty();
    if has_tools {
        obj.insert("tools".to_owned(), Value::Array(function_tools));
    }
    if let Some(tc) = &req.tool_choice {
        obj.insert("tool_choice".to_owned(), tool_choice_to_openai(tc));
    }
    if let Some(parallel) = req.parallel_tool_calls {
        if has_tools {
            obj.insert("parallel_tool_calls".to_owned(), json!(parallel));
        }
    }
    if let Some(rf) = &req.response_format {
        obj.insert("response_format".to_owned(), response_format_to_openai(rf));
    }
    if let Some(r) = &req.reasoning {
        obj.insert(
            "reasoning_effort".to_owned(),
            json!(r.derived_effort().as_str()),
        );
    }
    if let Some(m) = req.max_tokens {
        obj.insert("max_tokens".to_owned(), json!(m));
    }
    if let Some(t) = req.temperature {
        obj.insert("temperature".to_owned(), json!(t));
    }
    if let Some(p) = req.top_p {
        obj.insert("top_p".to_owned(), json!(p));
    }
    if !req.stop.is_empty() {
        obj.insert("stop".to_owned(), json!(req.stop));
    }
    if req.stream {
        obj.insert("stream".to_owned(), json!(true));
        obj.insert("stream_options".to_owned(), json!({"include_usage": true}));
    }
    Value::Object(obj)
}

pub(super) fn render_response(resp: &ChatResponse, model: &str) -> Value {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in &resp.content {
        match block {
            ContentBlock::Text { text: t } => text.push_str(t),
            ContentBlock::Thinking { text: t, .. } => reasoning.push_str(t),
            ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": value_to_args(input)},
                }));
            }
            _ => {}
        }
    }

    let mut message = Map::new();
    message.insert("role".to_owned(), json!("assistant"));
    message.insert(
        "content".to_owned(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    if !reasoning.is_empty() {
        message.insert("reasoning_content".to_owned(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }

    let id = if resp.id.is_empty() {
        "chatcmpl-litellm".to_owned()
    } else {
        resp.id.clone()
    };
    json!({
        "id": id,
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": resp
                .stop_reason
                .as_ref()
                .map(StopReason::to_openai)
                .unwrap_or_else(|| "stop".to_owned()),
        }],
        "usage": openai_usage(&resp.usage),
    })
}

pub(crate) fn source_to_data_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Url(url) => url.clone(),
        ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
    }
}

/// Flatten one IR message into one or more OpenAI messages. Tool results become
/// separate `role:"tool"` messages; tool uses become `tool_calls` on assistant.
pub(super) fn flatten_message(msg: &Message, out: &mut Vec<Value>) {
    // Tool-result blocks always become standalone tool messages, regardless of
    // the IR role they were grouped under (Anthropic nests them in user turns).
    for block in &msg.content {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = block
        {
            out.push(json!({
                "role": "tool",
                "tool_call_id": tool_use_id,
                "content": join_text(content),
            }));
        }
    }

    match msg.role {
        Role::Assistant => flatten_assistant(msg, out),
        _ => {
            let content = render_user_content(&msg.content);
            if !is_empty_content(&content) {
                out.push(json!({"role": "user", "content": content}));
            }
        }
    }
}

fn flatten_assistant(msg: &Message, out: &mut Vec<Value>) {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in &msg.content {
        match block {
            ContentBlock::Text { text: t } => text.push_str(t),
            ContentBlock::ToolUse { id, name, input } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": value_to_args(input)},
            })),
            _ => {}
        }
    }
    let mut m = Map::new();
    m.insert("role".to_owned(), json!("assistant"));
    m.insert(
        "content".to_owned(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    if !tool_calls.is_empty() {
        m.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }
    // Skip an entirely empty assistant turn.
    if !text.is_empty() || m.contains_key("tool_calls") {
        out.push(Value::Object(m));
    }
}

fn render_user_content(blocks: &[ContentBlock]) -> Value {
    let has_image = blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    if !has_image {
        return json!(join_text(blocks));
    }
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text } => parts.push(json!({"type": "text", "text": text})),
            ContentBlock::Image { source } => parts.push(json!({
                "type": "image_url",
                "image_url": {"url": source_to_data_url(source)},
            })),
            _ => {}
        }
    }
    Value::Array(parts)
}

fn is_empty_content(v: &Value) -> bool {
    match v {
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => true,
    }
}

pub(crate) fn join_text(blocks: &[ContentBlock]) -> String {
    let mut text = String::new();
    for block in blocks {
        if let ContentBlock::Text { text: t } = block {
            text.push_str(t);
        }
    }
    text
}

pub(crate) fn value_to_args(input: &Value) -> String {
    match input {
        Value::Null => "{}".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub(super) fn response_format_to_openai(rf: &ResponseFormat) -> Value {
    match rf {
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => json!({
            "type": "json_schema",
            "json_schema": {"name": name, "schema": schema, "strict": strict},
        }),
    }
}

pub(super) fn tool_to_openai(tool: &ToolDef) -> Value {
    let mut func = json!({"name": tool.name, "parameters": tool.parameters});
    if let Some(desc) = &tool.description {
        func["description"] = json!(desc);
    }
    json!({"type": "function", "function": func})
}

pub(super) fn tool_choice_to_openai(tc: &ToolChoice) -> Value {
    match tc {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool(name) => json!({"type": "function", "function": {"name": name}}),
    }
}

/// Build an OpenAI Chat `usage` object, adding `prompt_tokens_details.cached_tokens`
/// only when a cache hit occurred (keeps zero-cache output byte-identical).
pub(crate) fn openai_usage(u: &Usage) -> Value {
    let mut usage = json!({
        "prompt_tokens": u.input_tokens,
        "completion_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
    });
    if u.cache_read_input_tokens > 0 {
        usage["prompt_tokens_details"] = json!({"cached_tokens": u.cache_read_input_tokens});
    }
    usage
}
