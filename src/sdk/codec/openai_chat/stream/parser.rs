//! Streaming parser: OpenAI SSE chunks into IR `StreamEvent`s.

use std::collections::HashMap;

use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{BlockStart, StopReason, StreamEvent, Usage},
        stream::{SseEvent, StreamParser},
    },
};

use crate::sdk::codec::openai_chat::parse::usage_from_openai;

#[derive(Default)]
pub(in crate::sdk::codec::openai_chat) struct OpenAiChatStreamParser {
    started: bool,
    text_index: Option<usize>,
    think_index: Option<usize>,
    tool_indices: HashMap<u64, usize>,
    next_index: usize,
    stop_reason: Option<StopReason>,
    usage: Option<Usage>,
    blocks_closed: bool,
    delta_emitted: bool,
    message_stopped: bool,
}

impl OpenAiChatStreamParser {
    fn alloc(&mut self) -> usize {
        let i = self.next_index;
        self.next_index += 1;
        i
    }

    fn open_indices(&self) -> Vec<usize> {
        let mut idxs: Vec<usize> = self
            .text_index
            .into_iter()
            .chain(self.think_index)
            .chain(self.tool_indices.values().copied())
            .collect();
        idxs.sort_unstable();
        idxs
    }

    fn finalize(&mut self) -> Vec<StreamEvent> {
        if !self.started {
            return Vec::new();
        }
        let mut out = Vec::new();
        if !self.blocks_closed {
            self.blocks_closed = true;
            for index in self.open_indices() {
                out.push(StreamEvent::ContentBlockStop { index });
            }
        }
        if !self.delta_emitted {
            self.delta_emitted = true;
            out.push(StreamEvent::MessageDelta {
                stop_reason: self.stop_reason.take(),
                usage: self.usage.take(),
            });
        }
        if !self.message_stopped {
            self.message_stopped = true;
            out.push(StreamEvent::MessageStop);
        }
        out
    }

    fn message_start(&mut self, data: &Value) -> StreamEvent {
        self.started = true;
        StreamEvent::MessageStart {
            id: data
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model: data
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }

    fn handle_text(&mut self, delta: Option<&Value>, out: &mut Vec<StreamEvent>) {
        let Some(text) = delta.and_then(|d| d.get("content")).and_then(Value::as_str) else {
            return;
        };
        if text.is_empty() {
            return;
        }
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

    fn handle_reasoning(&mut self, delta: Option<&Value>, out: &mut Vec<StreamEvent>) {
        let Some(rt) = delta
            .and_then(|d| d.get("reasoning_content"))
            .and_then(Value::as_str)
        else {
            return;
        };
        if rt.is_empty() {
            return;
        }
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
            text: rt.to_owned(),
        });
    }

    fn handle_tool_calls(&mut self, delta: Option<&Value>, out: &mut Vec<StreamEvent>) {
        let Some(tcs) = delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for tc in tcs {
            let index = self.tool_index_for(tc, out);
            if let Some(args) = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
            {
                if !args.is_empty() {
                    out.push(StreamEvent::ToolUseInputDelta {
                        index,
                        partial_json: args.to_owned(),
                    });
                }
            }
        }
    }

    fn tool_index_for(&mut self, tc: &Value, out: &mut Vec<StreamEvent>) -> usize {
        let j = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
        match self.tool_indices.get(&j) {
            Some(i) => *i,
            None => {
                let i = self.alloc();
                self.tool_indices.insert(j, i);
                out.push(StreamEvent::ContentBlockStart {
                    index: i,
                    block: BlockStart::ToolUse {
                        id: tc
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    },
                });
                i
            }
        }
    }
}

impl StreamParser for OpenAiChatStreamParser {
    fn push(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, GatewayError> {
        if event.data.trim() == "[DONE]" {
            return Ok(self.finalize());
        }
        if event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| GatewayError::InvalidJsonMessage(e.to_string()))?;

        let mut out = Vec::new();
        if let Some(u) = data.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(usage_from_openai(Some(u)));
        }

        if !self.started {
            let ev = self.message_start(&data);
            out.push(ev);
        }

        // A streamed error arrives as a top-level `error` object with no choices;
        // surface it as an error stop so finish() emits a failure, not a clean stop.
        if let Some(err) = data.get("error") {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("stream error");
            self.stop_reason = Some(StopReason::Other(format!("error: {message}")));
            return Ok(out);
        }

        let Some(choices) = data.get("choices").and_then(Value::as_array) else {
            return Ok(out);
        };
        for choice in choices {
            let delta = choice.get("delta");
            self.handle_text(delta, &mut out);
            self.handle_reasoning(delta, &mut out);
            self.handle_tool_calls(delta, &mut out);
            if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                self.stop_reason = Some(StopReason::from_openai(fr));
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        self.finalize()
    }
}
