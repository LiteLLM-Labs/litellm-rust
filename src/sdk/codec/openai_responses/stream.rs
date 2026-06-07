//! Streaming parser/renderer for the Responses codec.

use std::collections::HashSet;

use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::codec::{
        ir::{BlockStart, StopReason, StreamEvent, Usage},
        stream::{SseEvent, StreamParser},
    },
};

use super::parse::usage_from_responses;

#[derive(Default)]
pub(super) struct ResponsesStreamParser {
    started: bool,
    opened: HashSet<usize>,
    saw_tool: bool,
    stop_reason: Option<StopReason>,
    usage: Option<Usage>,
    message_stopped: bool,
}

impl ResponsesStreamParser {
    fn finalize(&mut self) -> Vec<StreamEvent> {
        if !self.started || self.message_stopped {
            return Vec::new();
        }
        self.message_stopped = true;
        let mut out = Vec::new();
        let mut open: Vec<usize> = self.opened.drain().collect();
        open.sort_unstable();
        for index in open {
            out.push(StreamEvent::ContentBlockStop { index });
        }
        let stop = self.stop_reason.take().or(if self.saw_tool {
            Some(StopReason::ToolUse)
        } else {
            Some(StopReason::EndTurn)
        });
        out.push(StreamEvent::MessageDelta {
            stop_reason: stop,
            usage: self.usage.take(),
        });
        out.push(StreamEvent::MessageStop);
        out
    }

    fn on_created(data: &Value) -> Vec<StreamEvent> {
        let resp = data.get("response");
        vec![StreamEvent::MessageStart {
            id: resp
                .and_then(|r| r.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model: resp
                .and_then(|r| r.get("model"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }]
    }

    fn on_item_added(&mut self, data: &Value) -> Vec<StreamEvent> {
        let oi = output_index(data);
        let item = data.get("item");
        match item.and_then(|i| i.get("type")).and_then(Value::as_str) {
            Some("function_call") => {
                self.saw_tool = true;
                self.opened.insert(oi);
                vec![StreamEvent::ContentBlockStart {
                    index: oi,
                    block: BlockStart::ToolUse {
                        id: item
                            .and_then(|i| i.get("call_id"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: item
                            .and_then(|i| i.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    },
                }]
            }
            Some("reasoning") => {
                self.opened.insert(oi);
                vec![StreamEvent::ContentBlockStart {
                    index: oi,
                    block: BlockStart::Thinking,
                }]
            }
            _ => Vec::new(),
        }
    }

    fn on_part_added(&mut self, data: &Value) -> Vec<StreamEvent> {
        let oi = output_index(data);
        let is_text = data
            .get("part")
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            == Some("output_text");
        if is_text && self.opened.insert(oi) {
            vec![StreamEvent::ContentBlockStart {
                index: oi,
                block: BlockStart::Text,
            }]
        } else {
            Vec::new()
        }
    }

    fn on_item_done(&mut self, data: &Value) -> Vec<StreamEvent> {
        let oi = output_index(data);
        if self.opened.remove(&oi) {
            vec![StreamEvent::ContentBlockStop { index: oi }]
        } else {
            Vec::new()
        }
    }

    fn on_completion(&mut self, t: &str, data: &Value) -> Vec<StreamEvent> {
        if t == "response.incomplete" {
            self.stop_reason = Some(StopReason::MaxTokens);
        }
        self.usage = Some(usage_from_responses(
            data.get("response").and_then(|r| r.get("usage")),
        ));
        self.finalize()
    }
}

fn output_index(data: &Value) -> usize {
    data.get("output_index")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

impl StreamParser for ResponsesStreamParser {
    fn push(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, GatewayError> {
        if event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| GatewayError::InvalidJsonMessage(e.to_string()))?;
        let t = data.get("type").and_then(Value::as_str).unwrap_or_default();

        Ok(match t {
            "response.created" => {
                self.started = true;
                Self::on_created(&data)
            }
            "response.output_item.added" => self.on_item_added(&data),
            "response.content_part.added" => self.on_part_added(&data),
            "response.output_text.delta" => vec![StreamEvent::TextDelta {
                index: output_index(&data),
                text: delta_str(&data),
            }],
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                vec![StreamEvent::ThinkingDelta {
                    index: output_index(&data),
                    text: delta_str(&data),
                }]
            }
            "response.function_call_arguments.delta" => vec![StreamEvent::ToolUseInputDelta {
                index: output_index(&data),
                partial_json: delta_str(&data),
            }],
            "response.output_item.done" => self.on_item_done(&data),
            "response.completed" | "response.incomplete" | "response.failed" => {
                self.on_completion(t, &data)
            }
            _ => Vec::new(),
        })
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        self.finalize()
    }
}

fn delta_str(data: &Value) -> String {
    data.get("delta")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
