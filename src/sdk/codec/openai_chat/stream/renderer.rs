//! Streaming renderer: IR `StreamEvent`s into OpenAI SSE chunks.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::sdk::codec::{
    ir::{BlockStart, StopReason, StreamEvent, Usage},
    stream::{sse_frame, StreamRenderer},
};

use crate::sdk::codec::openai_chat::render::openai_usage;

pub(in crate::sdk::codec::openai_chat) struct OpenAiChatStreamRenderer {
    pub(in crate::sdk::codec::openai_chat) model: String,
    pub(in crate::sdk::codec::openai_chat) id: String,
    pub(in crate::sdk::codec::openai_chat) role_sent: bool,
    pub(in crate::sdk::codec::openai_chat) tool_index: HashMap<usize, usize>,
    pub(in crate::sdk::codec::openai_chat) next_tool: usize,
    pub(in crate::sdk::codec::openai_chat) done_sent: bool,
}

impl OpenAiChatStreamRenderer {
    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Vec<u8> {
        let data = json!({
            "id": if self.id.is_empty() { "chatcmpl-litellm" } else { &self.id },
            "object": "chat.completion.chunk",
            "created": 0,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason,
            }],
        });
        sse_frame(None, &data.to_string())
    }

    fn render_message_start(&mut self, id: &str) -> Vec<u8> {
        self.id = if id.is_empty() {
            "chatcmpl-litellm".to_owned()
        } else {
            id.to_owned()
        };
        self.role_sent = true;
        self.chunk(json!({"role": "assistant", "content": ""}), None)
    }

    fn render_tool_start(&mut self, index: usize, id: &str, name: &str) -> Vec<u8> {
        let j = self.next_tool;
        self.next_tool += 1;
        self.tool_index.insert(index, j);
        self.chunk(
            json!({"tool_calls": [{
                "index": j,
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": ""},
            }]}),
            None,
        )
    }

    fn render_tool_delta(&self, index: usize, partial_json: &str) -> Vec<u8> {
        let j = self.tool_index.get(&index).copied().unwrap_or(0);
        self.chunk(
            json!({"tool_calls": [{
                "index": j,
                "function": {"arguments": partial_json},
            }]}),
            None,
        )
    }

    fn render_message_delta(
        &self,
        stop_reason: &Option<StopReason>,
        usage: &Option<Usage>,
    ) -> Vec<u8> {
        let reason = stop_reason
            .as_ref()
            .map(StopReason::to_openai)
            .unwrap_or_else(|| "stop".to_owned());
        let mut out = self.chunk(json!({}), Some(&reason));
        if let Some(u) = usage {
            let data = json!({
                "id": if self.id.is_empty() { "chatcmpl-litellm" } else { &self.id },
                "object": "chat.completion.chunk",
                "created": 0,
                "model": self.model,
                "choices": [],
                "usage": openai_usage(u),
            });
            out.extend(sse_frame(None, &data.to_string()));
        }
        out
    }
}

impl StreamRenderer for OpenAiChatStreamRenderer {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8> {
        match event {
            StreamEvent::MessageStart { id, .. } => self.render_message_start(id),
            StreamEvent::ContentBlockStart {
                index,
                block: BlockStart::ToolUse { id, name },
            } => self.render_tool_start(*index, id, name),
            StreamEvent::ContentBlockStart { .. } => Vec::new(),
            StreamEvent::TextDelta { text, .. } => self.chunk(json!({"content": text}), None),
            StreamEvent::ThinkingDelta { text, .. } => {
                self.chunk(json!({"reasoning_content": text}), None)
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => self.render_tool_delta(*index, partial_json),
            StreamEvent::ContentBlockStop { .. } => Vec::new(),
            StreamEvent::MessageDelta { stop_reason, usage } => {
                self.render_message_delta(stop_reason, usage)
            }
            StreamEvent::MessageStop => {
                self.done_sent = true;
                sse_frame(None, "[DONE]")
            }
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.done_sent {
            Vec::new()
        } else {
            self.done_sent = true;
            sse_frame(None, "[DONE]")
        }
    }
}
