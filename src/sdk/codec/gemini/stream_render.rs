//! Streaming Gemini renderer: IR `StreamEvent` → `streamGenerateContent` SSE.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::common::gemini_usage;
use crate::sdk::codec::{
    ir::{BlockStart, StopReason, StreamEvent, Usage},
    stream::{sse_frame, StreamRenderer},
};

#[derive(Default)]
pub(super) struct GeminiStreamRenderer {
    /// Buffered partial-json args for in-flight tool-use blocks, by index.
    tool_args: HashMap<usize, (String, String)>, // index -> (name, buffered json)
    stop_reason: Option<StopReason>,
    usage: Option<Usage>,
    sent_finish: bool,
}

impl GeminiStreamRenderer {
    fn chunk(parts: Vec<Value>, finish: Option<&str>, usage: Option<&Usage>) -> Vec<u8> {
        let mut candidate = json!({
            "content": {"role": "model", "parts": parts},
            "index": 0,
        });
        if let Some(f) = finish {
            candidate["finishReason"] = json!(f);
        }
        let mut data = json!({"candidates": [candidate]});
        if let Some(u) = usage {
            data["usageMetadata"] = gemini_usage(u);
        }
        sse_frame(None, &data.to_string())
    }

    fn render_block_stop(&mut self, index: &usize) -> Vec<u8> {
        if let Some((name, buf)) = self.tool_args.remove(index) {
            let args: Value = serde_json::from_str(&buf).unwrap_or_else(|_| json!({}));
            Self::chunk(
                vec![json!({"functionCall": {"name": name, "args": args}})],
                None,
                None,
            )
        } else {
            Vec::new()
        }
    }

    fn render_message_stop(&mut self) -> Vec<u8> {
        self.sent_finish = true;
        // A surfaced provider error becomes a Gemini error frame, not a finished candidate.
        if let Some(StopReason::Other(message)) = &self.stop_reason {
            let err = json!({"error": {"code": 502, "message": message, "status": "UNKNOWN"}});
            return sse_frame(None, &err.to_string());
        }
        let finish = self
            .stop_reason
            .as_ref()
            .map(StopReason::to_gemini)
            .unwrap_or_else(|| "STOP".to_owned());
        Self::chunk(Vec::new(), Some(&finish), self.usage.clone().as_ref())
    }
}

impl StreamRenderer for GeminiStreamRenderer {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8> {
        match event {
            StreamEvent::MessageStart { .. }
            | StreamEvent::ContentBlockStart {
                block: BlockStart::Text,
                ..
            } => Vec::new(),
            StreamEvent::ContentBlockStart {
                index,
                block: BlockStart::ToolUse { name, .. },
            } => {
                self.tool_args.insert(*index, (name.clone(), String::new()));
                Vec::new()
            }
            StreamEvent::ContentBlockStart {
                block: BlockStart::Thinking,
                ..
            } => Vec::new(),
            StreamEvent::TextDelta { text, .. } => {
                Self::chunk(vec![json!({"text": text})], None, None)
            }
            StreamEvent::ThinkingDelta { text, .. } => {
                Self::chunk(vec![json!({"text": text, "thought": true})], None, None)
            }
            StreamEvent::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                if let Some((_, buf)) = self.tool_args.get_mut(index) {
                    buf.push_str(partial_json);
                }
                Vec::new()
            }
            StreamEvent::ContentBlockStop { index } => self.render_block_stop(index),
            StreamEvent::MessageDelta { stop_reason, usage } => {
                self.stop_reason = stop_reason.clone();
                self.usage = usage.clone();
                Vec::new()
            }
            StreamEvent::MessageStop => self.render_message_stop(),
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.sent_finish {
            Vec::new()
        } else {
            self.sent_finish = true;
            Self::chunk(Vec::new(), Some("STOP"), None)
        }
    }
}
