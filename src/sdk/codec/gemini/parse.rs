//! Wire → IR: parse Gemini requests and responses.

use serde_json::{json, Map, Value};

use super::common::{field, parse_parts_as_text, usage_from_gemini};
use super::parts::part_to_block;
use crate::{
    errors::GatewayError,
    sdk::codec::ir::{
        CacheMarkers, ChatRequest, ChatResponse, ContentBlock, Message, ReasoningConfig,
        ResponseFormat, Role, StopReason, ToolChoice, ToolDef,
    },
};

pub(super) fn parse_request(body: Value) -> Result<ChatRequest, GatewayError> {
    let obj = body.as_object().ok_or_else(|| {
        GatewayError::InvalidJsonMessage("request body must be a JSON object".to_owned())
    })?;

    let system = field(obj, "systemInstruction", "system_instruction")
        .map(parse_parts_as_text)
        .filter(|t| !t.is_empty())
        .map(|t| vec![ContentBlock::Text { text: t }])
        .unwrap_or_default();

    let mut messages: Vec<Message> = obj
        .get("contents")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(content_to_message).collect())
        .unwrap_or_default();
    // functionResponse parts carry only the name; realign each to its call's
    // surrogate id so cross-protocol tool_use/tool_result pairs stay matched.
    align_tool_result_ids(&mut messages);

    let tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .map(|arr| tools_from_gemini(arr))
        .unwrap_or_default();

    let tool_choice = parse_request_tool_choice(obj);
    let gen = field(obj, "generationConfig", "generation_config").and_then(Value::as_object);

    Ok(ChatRequest {
        model: String::new(),
        system,
        messages,
        tools,
        // Gemini implicit caching is automatic; nothing to carry from the wire.
        cache: CacheMarkers::default(),
        tool_choice,
        parallel_tool_calls: None,
        response_format: gen.and_then(parse_response_format),
        reasoning: gen.and_then(parse_reasoning),
        max_tokens: gen
            .and_then(|g| field(g, "maxOutputTokens", "max_output_tokens"))
            .and_then(Value::as_u64),
        temperature: gen
            .and_then(|g| g.get("temperature"))
            .and_then(Value::as_f64),
        top_p: gen
            .and_then(|g| field(g, "topP", "top_p"))
            .and_then(Value::as_f64),
        stop: gen.map(parse_stop).unwrap_or_default(),
        stream: false,
        extra: Map::new(),
    })
}

/// Rewrite each `ToolResult.tool_use_id` (set to the function name) to the id of
/// the matching `ToolUse`, FIFO per name, so parallel same-name calls stay paired
/// when re-rendered to providers that key tool results by id (Anthropic/OpenAI).
fn align_tool_result_ids(messages: &mut [Message]) {
    use std::collections::{HashMap, VecDeque};
    let mut calls: HashMap<String, VecDeque<String>> = HashMap::new();
    for msg in messages.iter_mut() {
        for block in msg.content.iter_mut() {
            match block {
                ContentBlock::ToolUse { id, name, .. } => {
                    calls.entry(name.clone()).or_default().push_back(id.clone());
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    if let Some(id) = calls.get_mut(tool_use_id).and_then(VecDeque::pop_front) {
                        *tool_use_id = id;
                    }
                }
                _ => {}
            }
        }
    }
}

fn parse_request_tool_choice(obj: &Map<String, Value>) -> Option<ToolChoice> {
    field(obj, "toolConfig", "tool_config")
        .and_then(|tc| {
            field(
                tc.as_object()?,
                "functionCallingConfig",
                "function_calling_config",
            )
        })
        .and_then(parse_tool_choice)
}

fn parse_stop(gen: &Map<String, Value>) -> Vec<String> {
    field(gen, "stopSequences", "stop_sequences")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_response_format(gen: &Map<String, Value>) -> Option<ResponseFormat> {
    let mime = field(gen, "responseMimeType", "response_mime_type").and_then(Value::as_str);
    if mime != Some("application/json") {
        return None;
    }
    match field(gen, "responseJsonSchema", "response_json_schema")
        .or_else(|| field(gen, "responseSchema", "response_schema"))
        .cloned()
    {
        Some(schema) => Some(ResponseFormat::JsonSchema {
            name: "response".to_owned(),
            schema,
            strict: true,
        }),
        None => Some(ResponseFormat::JsonObject),
    }
}

fn parse_reasoning(gen: &Map<String, Value>) -> Option<ReasoningConfig> {
    let tc = field(gen, "thinkingConfig", "thinking_config").and_then(Value::as_object)?;
    let budget = field(tc, "thinkingBudget", "thinking_budget").and_then(Value::as_u64);
    budget.map(|b| ReasoningConfig {
        effort: None,
        budget_tokens: Some(b),
    })
}

pub(super) fn parse_response(body: Value) -> Result<ChatResponse, GatewayError> {
    let obj = body.as_object().ok_or_else(|| {
        GatewayError::InvalidJsonMessage("response body must be a JSON object".to_owned())
    })?;
    let candidate = obj
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|a| a.first());

    let mut content = Vec::new();
    let mut saw_tool = false;
    if let Some(parts) = candidate
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(block) = part_to_block(part) {
                if matches!(block, ContentBlock::ToolUse { .. }) {
                    saw_tool = true;
                }
                content.push(block);
            }
        }
    }

    let finish = candidate
        .and_then(|c| field(c.as_object()?, "finishReason", "finish_reason"))
        .and_then(Value::as_str);
    // A 200 with promptFeedback.blockReason and no candidates is a prompt blocked
    // before generation; surface it as a content filter, not an empty success.
    let blocked = obj
        .get("promptFeedback")
        .or_else(|| obj.get("prompt_feedback"))
        .and_then(Value::as_object)
        .and_then(|pf| field(pf, "blockReason", "block_reason"))
        .is_some();
    let stop_reason = if saw_tool {
        Some(StopReason::ToolUse)
    } else if blocked && candidate.is_none() {
        Some(StopReason::ContentFilter)
    } else {
        finish.map(StopReason::from_gemini)
    };

    Ok(ChatResponse {
        id: String::new(),
        model: obj
            .get("modelVersion")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content,
        stop_reason,
        usage: usage_from_gemini(field(obj, "usageMetadata", "usage_metadata")),
    })
}

fn content_to_message(v: &Value) -> Option<Message> {
    let obj = v.as_object()?;
    let role = match obj.get("role").and_then(Value::as_str) {
        Some("model") => Role::Assistant,
        _ => Role::User,
    };
    let content = obj
        .get("parts")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(part_to_block).collect())
        .unwrap_or_default();
    Some(Message { role, content })
}

const GEMINI_BUILTIN_TOOL_KEYS: &[&str] = &[
    "google_search",
    "googleSearch",
    "code_execution",
    "codeExecution",
    "url_context",
    "urlContext",
    "google_search_retrieval",
    "googleSearchRetrieval",
];

fn tools_from_gemini(arr: &[Value]) -> Vec<ToolDef> {
    let mut tools = Vec::new();
    for entry in arr {
        push_function_decls(entry, &mut tools);
        push_builtin_tools(entry, &mut tools);
    }
    tools
}

fn push_function_decls(entry: &Value, tools: &mut Vec<ToolDef>) {
    let decls = entry
        .get("functionDeclarations")
        .or_else(|| entry.get("function_declarations"))
        .and_then(Value::as_array);
    let Some(decls) = decls else {
        return;
    };
    for d in decls {
        if let Some(name) = d.get("name").and_then(Value::as_str) {
            tools.push(ToolDef {
                name: name.to_owned(),
                description: d
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parameters: d
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"})),
                builtin: None,
            });
        }
    }
}

fn push_builtin_tools(entry: &Value, tools: &mut Vec<ToolDef>) {
    // Built-in / grounding tools (google_search, code_execution, …).
    for key in GEMINI_BUILTIN_TOOL_KEYS {
        if entry.get(*key).is_some() {
            tools.push(ToolDef {
                name: (*key).to_owned(),
                description: None,
                parameters: json!({"type": "object"}),
                builtin: Some(entry.clone()),
            });
        }
    }
}

fn parse_tool_choice(cfg: &Value) -> Option<ToolChoice> {
    let mode = cfg.get("mode").and_then(Value::as_str)?;
    match mode.to_ascii_uppercase().as_str() {
        "AUTO" => Some(ToolChoice::Auto),
        "NONE" => Some(ToolChoice::None),
        "ANY" => cfg
            .get("allowedFunctionNames")
            .or_else(|| cfg.get("allowed_function_names"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(|n| ToolChoice::Tool(n.to_owned()))
            .or(Some(ToolChoice::Required)),
        _ => None,
    }
}
