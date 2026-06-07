//! Request/response rendering for the Responses codec.

use serde_json::{json, Map, Value};

use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{
            ChatRequest, ChatResponse, ContentBlock, ImageSource, Message, ResponseFormat, Role,
            StopReason, ToolChoice, ToolDef, Usage,
        },
        openai_chat::{join_text, source_to_data_url, value_to_args},
        RequestCtx,
    },
};

pub(super) fn render_request(req: &ChatRequest) -> Result<Value, GatewayError> {
    let mut obj = Map::new();
    obj.insert("model".to_owned(), json!(req.model));
    if !req.system.is_empty() {
        obj.insert("instructions".to_owned(), json!(join_text(&req.system)));
    }
    let mut input: Vec<Value> = Vec::new();
    for msg in &req.messages {
        flatten_message(msg, &mut input);
    }
    obj.insert("input".to_owned(), Value::Array(input));
    let function_tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| t.builtin.is_none())
        .map(tool_to_responses)
        .collect();
    let has_tools = !function_tools.is_empty();
    if has_tools {
        obj.insert("tools".to_owned(), Value::Array(function_tools));
    }
    // Only send tool_choice when a function tool survived: built-in tools are
    // filtered out, and a choice naming a tool absent from the request is rejected.
    if has_tools {
        if let Some(tc) = &req.tool_choice {
            obj.insert("tool_choice".to_owned(), tool_choice_to_responses(tc));
        }
    }
    if let Some(parallel) = req.parallel_tool_calls {
        if has_tools {
            obj.insert("parallel_tool_calls".to_owned(), json!(parallel));
        }
    }
    if let Some(rf) = &req.response_format {
        obj.insert(
            "text".to_owned(),
            json!({"format": response_format_to_responses(rf)}),
        );
    }
    if let Some(r) = &req.reasoning {
        obj.insert(
            "reasoning".to_owned(),
            json!({"effort": r.derived_effort().as_str(), "summary": "auto"}),
        );
    }
    if let Some(m) = req.max_tokens {
        obj.insert("max_output_tokens".to_owned(), json!(m));
    }
    if let Some(t) = req.temperature {
        obj.insert("temperature".to_owned(), json!(t));
    }
    if let Some(p) = req.top_p {
        obj.insert("top_p".to_owned(), json!(p));
    }
    if req.stream {
        obj.insert("stream".to_owned(), json!(true));
    }
    Ok(Value::Object(obj))
}

pub(super) fn render_response(
    resp: &ChatResponse,
    ctx: &RequestCtx,
) -> Result<Value, GatewayError> {
    let mut output: Vec<Value> = Vec::new();
    let mut text = String::new();
    for block in &resp.content {
        match block {
            ContentBlock::Text { text: t } => text.push_str(t),
            ContentBlock::ToolUse { id, name, input } => output.push(json!({
                "type": "function_call",
                "id": format!("fc_{id}"),
                "call_id": id,
                "name": name,
                "arguments": value_to_args(input),
            })),
            ContentBlock::Thinking { text: t, .. } => output.push(json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": t}],
            })),
            _ => {}
        }
    }
    if !text.is_empty() {
        // Message item first to match OpenAI ordering.
        output.insert(
            0,
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}],
            }),
        );
    }

    let id = if resp.id.is_empty() {
        "resp_litellm".to_owned()
    } else {
        resp.id.clone()
    };
    let status = match resp.stop_reason {
        Some(StopReason::MaxTokens) => "incomplete",
        _ => "completed",
    };
    Ok(json!({
        "id": id,
        "object": "response",
        "model": ctx.model,
        "status": status,
        "output": output,
        "usage": responses_usage(&resp.usage),
    }))
}

fn flatten_message(msg: &Message, out: &mut Vec<Value>) {
    // Tool results become standalone function_call_output items.
    for block in &msg.content {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = block
        {
            out.push(json!({
                "type": "function_call_output",
                "call_id": tool_use_id,
                "output": join_text(content),
            }));
        }
    }

    match msg.role {
        Role::Assistant => {
            let mut text = String::new();
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text: t } => text.push_str(t),
                    ContentBlock::ToolUse { id, name, input } => out.push(json!({
                        "type": "function_call",
                        "call_id": id,
                        "name": name,
                        "arguments": value_to_args(input),
                    })),
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.push(json!({
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}],
                }));
            }
        }
        _ => {
            let mut parts: Vec<Value> = Vec::new();
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        parts.push(json!({"type": "input_text", "text": text}))
                    }
                    ContentBlock::Image { source } => parts.push(image_part(source)),
                    _ => {}
                }
            }
            if !parts.is_empty() {
                out.push(json!({"role": "user", "content": parts}));
            }
        }
    }
}

/// Render an IR image as a Responses `input_image` part (string `image_url`,
/// matching what the parser reads back).
fn image_part(source: &ImageSource) -> Value {
    json!({"type": "input_image", "image_url": source_to_data_url(source)})
}

fn response_format_to_responses(rf: &ResponseFormat) -> Value {
    match rf {
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => json!({
            "type": "json_schema",
            "name": name,
            "schema": schema,
            "strict": strict,
        }),
    }
}

fn tool_to_responses(tool: &ToolDef) -> Value {
    let mut o = json!({"type": "function", "name": tool.name, "parameters": tool.parameters});
    if let Some(desc) = &tool.description {
        o["description"] = json!(desc);
    }
    o
}

fn tool_choice_to_responses(tc: &ToolChoice) -> Value {
    match tc {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool(name) => json!({"type": "function", "name": name}),
    }
}

/// Build a Responses `usage` object, adding `input_tokens_details.cached_tokens`
/// only on a cache hit (keeps zero-cache output byte-identical).
pub(super) fn responses_usage(u: &Usage) -> Value {
    let mut usage = json!({
        "input_tokens": u.input_tokens,
        "output_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
    });
    if u.cache_read_input_tokens > 0 {
        usage["input_tokens_details"] = json!({"cached_tokens": u.cache_read_input_tokens});
    }
    usage
}
