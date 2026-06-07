//! Streaming Gemini → IR: `streamGenerateContent` parser.

use serde_json::{json, Value};

use super::common::usage_from_gemini;
use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{BlockStart, StopReason, StreamEvent, Usage},
        stream::{SseEvent, StreamParser},
    },
};

#[derive(Default)]
pub(super) struct GeminiStreamParser {
    started: bool,
    text_index: Option<usize>,
    think_index: Option<usize>,
    next_index: usize,
    saw_tool: bool,
    stop_reason: Option<StopReason>,
    usage: Option<Usage>,
    message_stopped: bool,
}

impl GeminiStreamParser {
    fn alloc(&mut self) -> usize {
        let i = self.next_index;
        self.next_index += 1;
        i
    }

    fn finalize(&mut self) -> Vec<StreamEvent> {
        if !self.started || self.message_stopped {
            return Vec::new();
        }
        self.message_stopped = true;
        let mut out = Vec::new();
        let mut open: Vec<usize> = self
            .text_index
            .take()
            .into_iter()
            .chain(self.think_index.take())
            .collect();
        open.sort_unstable();
        for index in open {
            out.push(StreamEvent::ContentBlockStop { index });
        }
        // Gemini reports finishReason STOP even when it emitted a functionCall, so
        // prefer ToolUse whenever a tool call was seen (mirrors parse_response).
        let stop_reason = if self.saw_tool {
            Some(StopReason::ToolUse)
        } else {
            self.stop_reason.take().or(Some(StopReason::EndTurn))
        };
        out.push(StreamEvent::MessageDelta {
            stop_reason,
            usage: self.usage.take(),
        });
        out.push(StreamEvent::MessageStop);
        out
    }

    fn maybe_start(&mut self, data: &Value, out: &mut Vec<StreamEvent>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(StreamEvent::MessageStart {
            id: String::new(),
            model: data
                .get("modelVersion")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }

    fn handle_part(&mut self, part: &Value, out: &mut Vec<StreamEvent>) {
        if let Some(fc) = part
            .get("functionCall")
            .or_else(|| part.get("function_call"))
        {
            self.handle_function_call(fc, out);
        } else if part.get("thought").and_then(Value::as_bool) == Some(true) {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                self.handle_thought(text, out);
            }
        } else if let Some(text) = part.get("text").and_then(Value::as_str) {
            self.handle_text(text, out);
        }
    }

    fn handle_function_call(&mut self, fc: &Value, out: &mut Vec<StreamEvent>) {
        self.saw_tool = true;
        let index = self.alloc();
        let name = fc.get("name").and_then(Value::as_str).unwrap_or_default();
        let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
        // Distinct id per call so parallel same-name calls don't collide for clients.
        let id = fc
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| super::parts::surrogate_id(name, &args, index));
        out.push(StreamEvent::ContentBlockStart {
            index,
            block: BlockStart::ToolUse {
                id,
                name: name.to_owned(),
            },
        });
        out.push(StreamEvent::ToolUseInputDelta {
            index,
            partial_json: args.to_string(),
        });
        out.push(StreamEvent::ContentBlockStop { index });
    }

    fn handle_thought(&mut self, text: &str, out: &mut Vec<StreamEvent>) {
        let index = match self.think_index {
            Some(i) => i,
            None => {
                let i = self.alloc();
                self.think_index = Some(i);
                out.push(StreamEvent::ContentBlockStart {
                    index: i,
                    block: BlockStart::Thinking,
                });
                i
            }
        };
        out.push(StreamEvent::ThinkingDelta {
            index,
            text: text.to_owned(),
        });
    }

    fn handle_text(&mut self, text: &str, out: &mut Vec<StreamEvent>) {
        let index = match self.text_index {
            Some(i) => i,
            None => {
                let i = self.alloc();
                self.text_index = Some(i);
                out.push(StreamEvent::ContentBlockStart {
                    index: i,
                    block: BlockStart::Text,
                });
                i
            }
        };
        out.push(StreamEvent::TextDelta {
            index,
            text: text.to_owned(),
        });
    }
}

impl StreamParser for GeminiStreamParser {
    fn push(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, GatewayError> {
        if event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| GatewayError::InvalidJsonMessage(e.to_string()))?;

        // A top-level error frame is a provider failure, not a candidate; surface it.
        if let Some(err) = data.get("error").filter(|e| !e.is_null()) {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("stream error");
            self.stop_reason = Some(StopReason::Other(format!("error: {message}")));
            // Mark started so finalize() emits the terminal even on an error-first frame.
            self.started = true;
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        self.maybe_start(&data, &mut out);

        if let Some(u) = data
            .get("usageMetadata")
            .or_else(|| data.get("usage_metadata"))
        {
            self.usage = Some(usage_from_gemini(Some(u)));
        }

        let candidate = data
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|a| a.first());
        if let Some(parts) = candidate
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                self.handle_part(part, &mut out);
            }
        }

        if let Some(fr) = candidate
            .and_then(|c| c.get("finishReason").or_else(|| c.get("finish_reason")))
            .and_then(Value::as_str)
        {
            self.stop_reason = Some(StopReason::from_gemini(fr));
        } else if candidate.is_none()
            && data
                .get("promptFeedback")
                .or_else(|| data.get("prompt_feedback"))
                .and_then(|pf| pf.get("blockReason").or_else(|| pf.get("block_reason")))
                .is_some()
        {
            // Blocked-prompt frame (no candidates): a content filter, not end-turn.
            self.stop_reason = Some(StopReason::ContentFilter);
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        self.finalize()
    }
}
