//! Streaming renderer for the Responses codec.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::sdk::codec::{
    ir::{BlockStart, StopReason, StreamEvent, Usage},
    stream::{sse_frame, StreamRenderer},
};

use super::render::responses_usage;

/// Accumulated content per output block, so `response.output_item.done` can carry
/// the completed item (text / reasoning / tool call) clients read the result from.
pub(super) enum ItemBuf {
    Message(String),
    Reasoning(String),
    FunctionCall {
        call_id: String,
        name: String,
        args: String,
    },
}

pub(super) struct ResponsesStreamRenderer {
    pub(super) model: String,
    pub(super) id: String,
    pub(super) next_oi: usize,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) usage: Option<Usage>,
    pub(super) items: HashMap<usize, ItemBuf>,
}

impl ResponsesStreamRenderer {
    fn item_id(oi: usize) -> String {
        format!("item_{oi}")
    }

    fn frame(t: &str, data: Value) -> Vec<u8> {
        sse_frame(Some(t), &data.to_string())
    }

    fn on_message_start(&mut self, id: &str) -> Vec<u8> {
        self.id = if id.is_empty() {
            "resp_litellm".to_owned()
        } else {
            id.to_owned()
        };
        Self::frame(
            "response.created",
            json!({
                "type": "response.created",
                "response": {"id": self.id, "object": "response", "model": self.model, "status": "in_progress"},
            }),
        )
    }

    fn on_block_start(&mut self, index: usize, block: &BlockStart) -> Vec<u8> {
        self.next_oi = self.next_oi.max(index + 1);
        let item_id = Self::item_id(index);
        self.items.insert(index, ItemBuf::for_block(block));
        match block {
            BlockStart::Text => {
                let mut out = Self::frame(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": index,
                        "item": {"type": "message", "id": item_id, "role": "assistant", "content": []},
                    }),
                );
                out.extend(Self::frame(
                    "response.content_part.added",
                    json!({
                        "type": "response.content_part.added",
                        "item_id": Self::item_id(index),
                        "output_index": index,
                        "content_index": 0,
                        "part": {"type": "output_text", "text": ""},
                    }),
                ));
                out
            }
            BlockStart::Thinking => Self::frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": index,
                    "item": {"type": "reasoning", "id": item_id, "summary": []},
                }),
            ),
            BlockStart::ToolUse { id, name } => Self::frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": index,
                    "item": {"type": "function_call", "id": item_id, "call_id": id, "name": name, "arguments": ""},
                }),
            ),
        }
    }

    fn on_message_stop(&self) -> Vec<u8> {
        let usage = self.usage.clone().unwrap_or_default();
        // A surfaced provider error (e.g. a translated Anthropic stream error)
        // must terminate as response.failed, not a clean response.completed.
        if let Some(StopReason::Other(message)) = &self.stop_reason {
            return Self::frame(
                "response.failed",
                json!({
                    "type": "response.failed",
                    "response": {
                        "id": self.id,
                        "object": "response",
                        "model": self.model,
                        "status": "failed",
                        "error": {"message": message},
                        "usage": responses_usage(&usage),
                    },
                }),
            );
        }
        // A truncated/filtered stream must terminate with response.incomplete (the
        // event type Responses clients dispatch on), not response.completed.
        let (event, reason) = match self.stop_reason {
            Some(StopReason::MaxTokens) => ("response.incomplete", Some("max_output_tokens")),
            Some(StopReason::ContentFilter) => ("response.incomplete", Some("content_filter")),
            _ => ("response.completed", None),
        };
        let status = if reason.is_some() {
            "incomplete"
        } else {
            "completed"
        };
        let mut response = json!({
            "id": self.id,
            "object": "response",
            "model": self.model,
            "status": status,
            "usage": responses_usage(&usage),
        });
        if let Some(reason) = reason {
            response["incomplete_details"] = json!({"reason": reason});
        }
        Self::frame(event, json!({"type": event, "response": response}))
    }

    fn on_text_delta(&mut self, index: usize, text: &str) -> Vec<u8> {
        if let Some(ItemBuf::Message(buf)) = self.items.get_mut(&index) {
            buf.push_str(text);
        }
        Self::frame(
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": Self::item_id(index),
                "output_index": index,
                "content_index": 0,
                "delta": text,
            }),
        )
    }

    fn on_thinking_delta(&mut self, index: usize, text: &str) -> Vec<u8> {
        if let Some(ItemBuf::Reasoning(buf)) = self.items.get_mut(&index) {
            buf.push_str(text);
        }
        Self::frame(
            "response.reasoning_summary_text.delta",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": Self::item_id(index),
                "output_index": index,
                "delta": text,
            }),
        )
    }

    fn on_tool_delta(&mut self, index: usize, partial: &str) -> Vec<u8> {
        if let Some(ItemBuf::FunctionCall { args, .. }) = self.items.get_mut(&index) {
            args.push_str(partial);
        }
        Self::frame(
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": Self::item_id(index),
                "output_index": index,
                "delta": partial,
            }),
        )
    }

    fn on_item_done(&mut self, index: usize) -> Vec<u8> {
        let item = self
            .items
            .remove(&index)
            .map(|buf| buf.into_item(&Self::item_id(index)))
            .unwrap_or(Value::Null);
        Self::frame(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": index,
                "item": item,
            }),
        )
    }
}

impl ItemBuf {
    fn for_block(block: &BlockStart) -> Self {
        match block {
            BlockStart::Text => Self::Message(String::new()),
            BlockStart::Thinking => Self::Reasoning(String::new()),
            BlockStart::ToolUse { id, name } => Self::FunctionCall {
                call_id: id.clone(),
                name: name.clone(),
                args: String::new(),
            },
        }
    }

    fn into_item(self, item_id: &str) -> Value {
        match self {
            Self::Message(text) => json!({
                "type": "message", "id": item_id, "role": "assistant",
                "status": "completed", "content": [{"type": "output_text", "text": text}],
            }),
            Self::Reasoning(text) => json!({
                "type": "reasoning", "id": item_id,
                "summary": [{"type": "summary_text", "text": text}],
            }),
            Self::FunctionCall {
                call_id,
                name,
                args,
            } => json!({
                "type": "function_call", "id": item_id, "call_id": call_id,
                "name": name, "arguments": args, "status": "completed",
            }),
        }
    }
}

impl StreamRenderer for ResponsesStreamRenderer {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8> {
        match event {
            StreamEvent::MessageStart { id, .. } => self.on_message_start(id),
            StreamEvent::ContentBlockStart { index, block } => self.on_block_start(*index, block),
            StreamEvent::TextDelta { index, text } => self.on_text_delta(*index, text),
            StreamEvent::ThinkingDelta { index, text } => self.on_thinking_delta(*index, text),
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => self.on_tool_delta(*index, partial_json),
            StreamEvent::ContentBlockStop { index } => self.on_item_done(*index),
            StreamEvent::MessageDelta { stop_reason, usage } => {
                self.stop_reason = stop_reason.clone();
                self.usage = usage.clone();
                Vec::new()
            }
            StreamEvent::MessageStop => self.on_message_stop(),
        }
    }
}
