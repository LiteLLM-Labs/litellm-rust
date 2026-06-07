//! Anthropic SSE stream parsing and rendering.

use serde_json::{json, Value};

use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{BlockStart, StopReason, StreamEvent, Usage},
        stream::{sse_frame, SseEvent, StreamParser, StreamRenderer},
    },
};

use super::blocks::usage_from_anthropic;

#[derive(Default)]
pub(super) struct AnthropicStreamParser {
    /// Usage from `message_start` (Anthropic reports cache tokens there). Folded
    /// into the final `message_delta` so downstream renderers see complete usage.
    start_usage: Usage,
}

impl StreamParser for AnthropicStreamParser {
    fn push(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, GatewayError> {
        let kind = event.event.as_deref().unwrap_or_default();
        if event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| GatewayError::InvalidJsonMessage(e.to_string()))?;
        Ok(match kind {
            "message_start" => self.parse_message_start(&data),
            "content_block_start" => parse_content_block_start(&data),
            "content_block_delta" => parse_content_block_delta(&data),
            "content_block_stop" => vec![StreamEvent::ContentBlockStop {
                index: index_of(&data),
            }],
            "message_delta" => self.parse_message_delta(&data),
            "message_stop" => vec![StreamEvent::MessageStop],
            _ => Vec::new(),
        })
    }
}

impl AnthropicStreamParser {
    fn parse_message_start(&mut self, data: &Value) -> Vec<StreamEvent> {
        let msg = data.get("message");
        self.start_usage = usage_from_anthropic(msg.and_then(|m| m.get("usage")));
        vec![StreamEvent::MessageStart {
            id: str_at(msg, "id"),
            model: str_at(msg, "model"),
        }]
    }

    fn parse_message_delta(&self, data: &Value) -> Vec<StreamEvent> {
        let stop_reason = data
            .get("delta")
            .and_then(|d| d.get("stop_reason"))
            .and_then(Value::as_str)
            .map(StopReason::from_anthropic);
        // The delta carries output_tokens; fold in input + cache tokens
        // captured from message_start so the emitted usage is complete.
        let usage = data.get("usage").map(|u| {
            let mut u = usage_from_anthropic(Some(u));
            u.input_tokens = u.input_tokens.max(self.start_usage.input_tokens);
            u.cache_read_input_tokens = self.start_usage.cache_read_input_tokens;
            u.cache_creation_input_tokens = self.start_usage.cache_creation_input_tokens;
            u
        });
        vec![StreamEvent::MessageDelta { stop_reason, usage }]
    }
}

fn parse_content_block_start(data: &Value) -> Vec<StreamEvent> {
    let index = index_of(data);
    let cb = data.get("content_block");
    let block = match cb.and_then(|c| c.get("type")).and_then(Value::as_str) {
        Some("tool_use") => BlockStart::ToolUse {
            id: str_at(cb, "id"),
            name: str_at(cb, "name"),
        },
        Some("thinking") => BlockStart::Thinking,
        _ => BlockStart::Text,
    };
    vec![StreamEvent::ContentBlockStart { index, block }]
}

fn parse_content_block_delta(data: &Value) -> Vec<StreamEvent> {
    let index = index_of(data);
    let delta = data.get("delta");
    match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
        Some("text_delta") => vec![StreamEvent::TextDelta {
            index,
            text: str_at(delta, "text"),
        }],
        Some("thinking_delta") => vec![StreamEvent::ThinkingDelta {
            index,
            text: str_at(delta, "thinking"),
        }],
        Some("input_json_delta") => vec![StreamEvent::ToolUseInputDelta {
            index,
            partial_json: str_at(delta, "partial_json"),
        }],
        _ => Vec::new(),
    }
}

fn index_of(data: &Value) -> usize {
    data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize
}

fn str_at(parent: Option<&Value>, key: &str) -> String {
    parent
        .and_then(|p| p.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) struct AnthropicStreamRenderer {
    pub(super) model: String,
}

impl StreamRenderer for AnthropicStreamRenderer {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8> {
        match event {
            StreamEvent::MessageStart { id, .. } => self.render_message_start(id),
            StreamEvent::ContentBlockStart { index, block } => {
                render_content_block_start(*index, block)
            }
            StreamEvent::TextDelta { index, text } => {
                render_delta(*index, json!({"type": "text_delta", "text": text}))
            }
            StreamEvent::ThinkingDelta { index, text } => {
                render_delta(*index, json!({"type": "thinking_delta", "thinking": text}))
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => render_delta(
                *index,
                json!({"type": "input_json_delta", "partial_json": partial_json}),
            ),
            StreamEvent::ContentBlockStop { index } => {
                let data = json!({"type": "content_block_stop", "index": index});
                sse_frame(Some("content_block_stop"), &data.to_string())
            }
            StreamEvent::MessageDelta { stop_reason, usage } => {
                render_message_delta(stop_reason.as_ref(), usage.as_ref())
            }
            StreamEvent::MessageStop => sse_frame(
                Some("message_stop"),
                &json!({"type": "message_stop"}).to_string(),
            ),
        }
    }
}

impl AnthropicStreamRenderer {
    fn render_message_start(&self, id: &str) -> Vec<u8> {
        let id = if id.is_empty() { "msg_litellm" } else { id };
        let data = json!({
            "type": "message_start",
            "message": {
                "id": id,
                "type": "message",
                "role": "assistant",
                "model": self.model,
                "content": [],
                "stop_reason": Value::Null,
                "usage": {"input_tokens": 0, "output_tokens": 0},
            }
        });
        sse_frame(Some("message_start"), &data.to_string())
    }
}

fn render_content_block_start(index: usize, block: &BlockStart) -> Vec<u8> {
    let content_block = match block {
        BlockStart::Text => json!({"type": "text", "text": ""}),
        BlockStart::Thinking => json!({"type": "thinking", "thinking": ""}),
        BlockStart::ToolUse { id, name } => {
            json!({"type": "tool_use", "id": id, "name": name, "input": {}})
        }
    };
    let data = json!({
        "type": "content_block_start",
        "index": index,
        "content_block": content_block,
    });
    sse_frame(Some("content_block_start"), &data.to_string())
}

fn render_delta(index: usize, delta: Value) -> Vec<u8> {
    let data = json!({
        "type": "content_block_delta",
        "index": index,
        "delta": delta,
    });
    sse_frame(Some("content_block_delta"), &data.to_string())
}

fn render_message_delta(stop_reason: Option<&StopReason>, usage: Option<&Usage>) -> Vec<u8> {
    let mut usage_obj = json!({"output_tokens": usage.map(|u| u.output_tokens).unwrap_or(0)});
    // Surface cache tokens when present (e.g. a non-Anthropic upstream
    // reported them at end-of-stream). Omitted when zero so the common
    // case stays byte-identical.
    if let Some(u) = usage {
        if u.cache_read_input_tokens > 0 || u.cache_creation_input_tokens > 0 {
            usage_obj["input_tokens"] = json!(u.non_cached_input_tokens());
            usage_obj["cache_read_input_tokens"] = json!(u.cache_read_input_tokens);
            usage_obj["cache_creation_input_tokens"] = json!(u.cache_creation_input_tokens);
        }
    }
    let data = json!({
        "type": "message_delta",
        "delta": {"stop_reason": stop_reason.map(StopReason::to_anthropic)},
        "usage": usage_obj,
    });
    sse_frame(Some("message_delta"), &data.to_string())
}
