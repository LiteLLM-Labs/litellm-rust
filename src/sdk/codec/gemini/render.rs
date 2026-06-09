//! IR → wire: render Gemini requests and responses.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::common::{gemini_usage, join_text};
use super::parts::{block_to_part, response_object};
use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{
            ChatRequest, ChatResponse, ContentBlock, Message, ResponseFormat, Role, StopReason,
            ToolChoice, ToolDef,
        },
        RequestCtx,
    },
};

pub(super) fn render_request(req: &ChatRequest) -> Result<Value, GatewayError> {
    let mut obj = Map::new();
    if !req.system.is_empty() {
        let text = join_text(&req.system);
        obj.insert(
            "systemInstruction".to_owned(),
            json!({"parts": [{"text": text}]}),
        );
    }

    let names = tool_name_map(&req.messages);
    obj.insert(
        "contents".to_owned(),
        Value::Array(build_contents(req, &names)),
    );

    let function_names: Vec<&str> = req
        .tools
        .iter()
        .filter(|t| t.builtin.is_none())
        .map(|t| t.name.as_str())
        .collect();
    let decls: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| t.builtin.is_none())
        .map(tool_to_gemini)
        .collect();
    let has_decls = !decls.is_empty();
    if has_decls {
        obj.insert(
            "tools".to_owned(),
            json!([{ "functionDeclarations": decls }]),
        );
    }
    // functionCallingConfig applies to declared functions; don't send it when no
    // function declaration survived or a named choice targets an absent one.
    if let Some(tc) = &req.tool_choice {
        if has_decls && tc.applies_to(&function_names) {
            obj.insert("toolConfig".to_owned(), tool_choice_to_gemini(tc));
        }
    }

    let gen = build_generation_config(req);
    if !gen.is_empty() {
        obj.insert("generationConfig".to_owned(), Value::Object(gen));
    }
    Ok(Value::Object(obj))
}

/// Coalesce consecutive same-role contents: Gemini expects user/model to
/// alternate, but parallel tool results arrive as separate IR `Role::Tool`
/// messages that all map to the "user" role.
fn build_contents(req: &ChatRequest, names: &HashMap<String, String>) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::new();
    for msg in &req.messages {
        let Some(content) = message_to_content(msg, names) else {
            continue;
        };
        match contents.last_mut() {
            Some(last) if last.get("role") == content.get("role") => {
                if let (Some(Value::Array(dst)), Some(Value::Array(src))) =
                    (last.get_mut("parts"), content.get("parts"))
                {
                    dst.extend(src.iter().cloned());
                }
            }
            _ => contents.push(content),
        }
    }
    contents
}

fn build_generation_config(req: &ChatRequest) -> Map<String, Value> {
    let mut gen = Map::new();
    if let Some(m) = req.max_tokens {
        gen.insert("maxOutputTokens".to_owned(), json!(m));
    }
    if let Some(t) = req.temperature {
        gen.insert("temperature".to_owned(), json!(t));
    }
    if let Some(p) = req.top_p {
        gen.insert("topP".to_owned(), json!(p));
    }
    if !req.stop.is_empty() {
        gen.insert("stopSequences".to_owned(), json!(req.stop));
    }
    if let Some(rf) = &req.response_format {
        gen.insert("responseMimeType".to_owned(), json!("application/json"));
        if let ResponseFormat::JsonSchema { schema, .. } = rf {
            gen.insert("responseJsonSchema".to_owned(), schema.clone());
        }
    }
    if let Some(r) = &req.reasoning {
        gen.insert(
            "thinkingConfig".to_owned(),
            json!({"thinkingBudget": r.derived_budget()}),
        );
    }
    gen
}

pub(super) fn render_response(
    resp: &ChatResponse,
    _ctx: &RequestCtx,
) -> Result<Value, GatewayError> {
    // A surfaced provider error is a Gemini error envelope, not a normal candidate.
    if let Some(StopReason::Other(message)) = &resp.stop_reason {
        return Ok(json!({"error": {"code": 502, "message": message, "status": "UNKNOWN"}}));
    }
    let parts: Vec<Value> = resp.content.iter().filter_map(block_to_part).collect();
    let finish = resp
        .stop_reason
        .as_ref()
        .map(StopReason::to_gemini)
        .unwrap_or_else(|| "STOP".to_owned());
    Ok(json!({
        "candidates": [{
            "content": {"role": "model", "parts": parts},
            "finishReason": finish,
            "index": 0,
        }],
        "usageMetadata": gemini_usage(&resp.usage),
    }))
}

/// Build one Gemini content per IR message. Tool results map to `functionResponse`
/// parts in a user-role content, keyed by name via `names`.
fn message_to_content(msg: &Message, names: &HashMap<String, String>) -> Option<Value> {
    let role = if msg.role == Role::Assistant {
        "model"
    } else {
        "user"
    };
    let mut parts: Vec<Value> = Vec::new();
    for block in &msg.content {
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                let name = names
                    .get(tool_use_id)
                    .cloned()
                    .unwrap_or_else(|| tool_use_id.clone());
                parts.push(json!({
                    "functionResponse": {"name": name, "response": response_object(content)},
                }));
            }
            other => {
                if let Some(part) = block_to_part(other) {
                    parts.push(part);
                }
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(json!({"role": role, "parts": parts}))
}

/// Map tool-use ids → function names so tool results can be rendered with the
/// name Gemini matches on.
fn tool_name_map(messages: &[Message]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for msg in messages {
        for block in &msg.content {
            if let ContentBlock::ToolUse { id, name, .. } = block {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

fn tool_to_gemini(tool: &ToolDef) -> Value {
    let mut o = json!({"name": tool.name, "parameters": tool.parameters});
    if let Some(desc) = &tool.description {
        o["description"] = json!(desc);
    }
    o
}

fn tool_choice_to_gemini(tc: &ToolChoice) -> Value {
    let cfg = match tc {
        ToolChoice::Auto => json!({"mode": "AUTO"}),
        ToolChoice::None => json!({"mode": "NONE"}),
        ToolChoice::Required => json!({"mode": "ANY"}),
        ToolChoice::Tool(name) => json!({"mode": "ANY", "allowedFunctionNames": [name]}),
    };
    json!({"functionCallingConfig": cfg})
}
