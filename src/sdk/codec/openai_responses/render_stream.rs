//! Streaming renderer for the Responses codec.

use serde_json::{json, Value};

use crate::sdk::codec::{
    ir::{BlockStart, StopReason, StreamEvent, Usage},
    stream::{sse_frame, StreamRenderer},
};

use super::render::responses_usage;

pub(super) struct ResponsesStreamRenderer {
    pub(super) model: String,
    pub(super) id: String,
    pub(super) next_oi: usize,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) usage: Option<Usage>,
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
        let status = match self.stop_reason {
            Some(StopReason::MaxTokens) => "incomplete",
            _ => "completed",
        };
        let usage = self.usage.clone().unwrap_or_default();
        Self::frame(
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {
                    "id": self.id,
                    "object": "response",
                    "model": self.model,
                    "status": status,
                    "usage": responses_usage(&usage),
                },
            }),
        )
    }
}

impl StreamRenderer for ResponsesStreamRenderer {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8> {
        match event {
            StreamEvent::MessageStart { id, .. } => self.on_message_start(id),
            StreamEvent::ContentBlockStart { index, block } => self.on_block_start(*index, block),
            StreamEvent::TextDelta { index, text } => Self::frame(
                "response.output_text.delta",
                json!({
                    "type": "response.output_text.delta",
                    "item_id": Self::item_id(*index),
                    "output_index": index,
                    "content_index": 0,
                    "delta": text,
                }),
            ),
            StreamEvent::ThinkingDelta { index, text } => Self::frame(
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": Self::item_id(*index),
                    "output_index": index,
                    "delta": text,
                }),
            ),
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => Self::frame(
                "response.function_call_arguments.delta",
                json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": Self::item_id(*index),
                    "output_index": index,
                    "delta": partial_json,
                }),
            ),
            StreamEvent::ContentBlockStop { index } => Self::frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": index,
                }),
            ),
            StreamEvent::MessageDelta { stop_reason, usage } => {
                self.stop_reason = stop_reason.clone();
                self.usage = usage.clone();
                Vec::new()
            }
            StreamEvent::MessageStop => self.on_message_stop(),
        }
    }
}
