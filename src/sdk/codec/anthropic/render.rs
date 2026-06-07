//! Rendering the IR back into Anthropic request/response JSON.

use serde_json::{json, Map, Value};

use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{ChatRequest, ChatResponse, Message, StopReason},
        RequestCtx,
    },
};

use super::blocks::{
    block_to_anthropic, set_cache_control, tool_choice_to_anthropic, tool_to_anthropic,
};
use super::DEFAULT_MAX_TOKENS;

pub(super) fn render_request(req: &ChatRequest) -> Result<Value, GatewayError> {
    let mut obj = Map::new();
    obj.insert("model".to_owned(), json!(req.model));
    obj.insert(
        "max_tokens".to_owned(),
        json!(req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    if let Some(system) = render_system(req) {
        obj.insert("system".to_owned(), system);
    }
    obj.insert(
        "messages".to_owned(),
        Value::Array(render_messages(&req.messages, &req.cache.messages)),
    );
    if let Some(tools) = render_tools(req) {
        obj.insert("tools".to_owned(), tools);
    }
    if let Some(choice) = render_tool_choice(req) {
        obj.insert("tool_choice".to_owned(), choice);
    }
    render_sampling(req, &mut obj);
    if !req.stop.is_empty() {
        obj.insert("stop_sequences".to_owned(), json!(req.stop));
    }
    if req.stream {
        obj.insert("stream".to_owned(), json!(true));
    }
    Ok(Value::Object(obj))
}

fn render_system(req: &ChatRequest) -> Option<Value> {
    if req.system.is_empty() {
        return None;
    }
    let mut system: Vec<Value> = req.system.iter().map(block_to_anthropic).collect();
    if req.cache.system {
        if let Some(last) = system.last_mut() {
            set_cache_control(last);
        }
    }
    Some(Value::Array(system))
}

fn render_tools(req: &ChatRequest) -> Option<Value> {
    // Built-in/server tools are origin-specific; drop them rather than
    // render a bogus function tool the other side can't satisfy.
    let mut function_tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| t.builtin.is_none())
        .map(tool_to_anthropic)
        .collect();
    if function_tools.is_empty() {
        return None;
    }
    if req.cache.tools {
        if let Some(last) = function_tools.last_mut() {
            set_cache_control(last);
        }
    }
    Some(Value::Array(function_tools))
}

fn render_tool_choice(req: &ChatRequest) -> Option<Value> {
    let tc = req.tool_choice.as_ref()?;
    let mut choice = tool_choice_to_anthropic(tc);
    if req.parallel_tool_calls == Some(false) {
        if let Some(o) = choice.as_object_mut() {
            o.insert("disable_parallel_tool_use".to_owned(), json!(true));
        }
    }
    Some(choice)
}

/// Extended thinking: Anthropic requires 1024 <= budget_tokens < max_tokens
/// and the default sampling temperature, so gate temperature/top_p on it.
fn render_sampling(req: &ChatRequest, obj: &mut Map<String, Value>) {
    let mt = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let thinking_on = match &req.reasoning {
        Some(r) => {
            let budget = r
                .budget_tokens
                .or_else(|| r.effort.map(|e| e.to_budget()))
                .unwrap_or(0);
            if budget >= 1024 && mt > 1024 {
                obj.insert(
                    "thinking".to_owned(),
                    json!({"type": "enabled", "budget_tokens": budget.min(mt - 1)}),
                );
                true
            } else {
                false
            }
        }
        None => false,
    };
    if !thinking_on {
        if let Some(t) = req.temperature {
            obj.insert("temperature".to_owned(), json!(t));
        }
        if let Some(p) = req.top_p {
            obj.insert("top_p".to_owned(), json!(p));
        }
    }
}

/// Render IR messages to Anthropic, coalescing consecutive same-role turns into
/// one. Anthropic requires user/assistant to alternate, but a single IR turn can
/// span several messages (e.g. parallel tool results arrive as separate
/// `Role::Tool` messages that all map to the "user" wire role). Empty-content
/// messages are dropped, since Anthropic rejects an empty `content` array.
fn render_messages(messages: &[Message], cache_idx: &[usize]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        let role = msg.role.as_anthropic();
        let blocks: Vec<Value> = msg.content.iter().map(block_to_anthropic).collect();
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.get("role").and_then(Value::as_str) == Some(role) => {
                if let Some(Value::Array(arr)) = last.get_mut("content") {
                    arr.extend(blocks);
                }
            }
            _ => out.push(json!({"role": role, "content": Value::Array(blocks)})),
        }
        // A cache breakpoint on this message lands on the last block we just
        // appended (its tail, even after coalescing into the previous turn).
        if cache_idx.contains(&i) {
            if let Some(Value::Array(arr)) = out.last_mut().and_then(|m| m.get_mut("content")) {
                if let Some(last_block) = arr.last_mut() {
                    set_cache_control(last_block);
                }
            }
        }
    }
    out
}

pub(super) fn render_response(
    resp: &ChatResponse,
    ctx: &RequestCtx,
) -> Result<Value, GatewayError> {
    let id = if resp.id.is_empty() {
        "msg_litellm".to_owned()
    } else {
        resp.id.clone()
    };
    Ok(json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": ctx.model,
        "content": Value::Array(resp.content.iter().map(block_to_anthropic).collect()),
        "stop_reason": resp.stop_reason.as_ref().map(StopReason::to_anthropic),
        "stop_sequence": Value::Null,
        "usage": {
            // Anthropic reports `input_tokens` as the post-breakpoint remainder.
            "input_tokens": resp.usage.non_cached_input_tokens(),
            "output_tokens": resp.usage.output_tokens,
            "cache_creation_input_tokens": resp.usage.cache_creation_input_tokens,
            "cache_read_input_tokens": resp.usage.cache_read_input_tokens,
        },
    }))
}
