use std::collections::HashMap;

use serde_json::{json, Value};

use crate::agents::{
    events,
    harnesses::{HarnessEvent, HarnessRunContext},
};

#[derive(Debug, Clone, Default)]
pub(super) struct ClaudeStreamTranslator {
    tool_names: HashMap<String, String>,
    tool_inputs: HashMap<String, String>,
}

impl ClaudeStreamTranslator {
    pub(super) fn map(&mut self, context: &HarnessRunContext, event: Value) -> Vec<HarnessEvent> {
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => self.block_start(context, &event),
            Some("content_block_delta") => self.block_delta(context, &event),
            Some("content_block_stop") => self.block_stop(context, &event),
            _ => Vec::new(),
        }
    }

    fn block_start(&mut self, context: &HarnessRunContext, event: &Value) -> Vec<HarnessEvent> {
        let Some(block) = event.get("content_block") else {
            return Vec::new();
        };
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        let part_id = part_id(context, index, block.get("type").and_then(Value::as_str));
        match block.get("type").and_then(Value::as_str) {
            Some("text") => vec![part_updated(
                context,
                &part_id,
                json!({ "type": "text", "text": block_text(block) }),
            )],
            Some("thinking") => vec![part_updated(
                context,
                &part_id,
                json!({ "type": "thinking", "text": block_text(block) }),
            )],
            Some("tool_use") => self.tool_start(context, &part_id, block),
            _ => Vec::new(),
        }
    }

    fn tool_start(
        &mut self,
        context: &HarnessRunContext,
        part_id: &str,
        block: &Value,
    ) -> Vec<HarnessEvent> {
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_owned();
        self.tool_names.insert(part_id.to_owned(), name.clone());
        vec![tool_updated(
            context,
            part_id,
            &name,
            "running",
            block.get("input").cloned().unwrap_or_else(|| json!({})),
        )]
    }

    fn block_delta(&mut self, context: &HarnessRunContext, event: &Value) -> Vec<HarnessEvent> {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(delta) = event.get("delta") else {
            return Vec::new();
        };
        let part_id = part_id(context, index, None);
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => delta_text(delta)
                .map(|text| vec![text_delta(context, &part_id, "text", text)])
                .unwrap_or_default(),
            Some("thinking_delta") => delta_text(delta)
                .map(|text| vec![text_delta(context, &part_id, "reasoning", text)])
                .unwrap_or_default(),
            Some("input_json_delta") => self.input_delta(context, &part_id, delta),
            _ => Vec::new(),
        }
    }

    fn input_delta(
        &mut self,
        context: &HarnessRunContext,
        part_id: &str,
        delta: &Value,
    ) -> Vec<HarnessEvent> {
        let partial = delta
            .get("partial_json")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.tool_inputs
            .entry(part_id.to_owned())
            .or_default()
            .push_str(partial);
        let input = parsed_input(self.tool_inputs.get(part_id).map(String::as_str));
        vec![tool_updated(
            context,
            part_id,
            self.tool_name(part_id).as_str(),
            "running",
            input,
        )]
    }

    fn block_stop(&mut self, context: &HarnessRunContext, event: &Value) -> Vec<HarnessEvent> {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        let part_id = part_id(context, index, None);
        let Some(name) = self.tool_names.remove(&part_id) else {
            return Vec::new();
        };
        let input = parsed_input(self.tool_inputs.remove(&part_id).as_deref());
        vec![tool_updated(context, &part_id, &name, "completed", input)]
    }

    fn tool_name(&self, part_id: &str) -> String {
        self.tool_names
            .get(part_id)
            .cloned()
            .unwrap_or_else(|| "tool".to_owned())
    }

    pub(super) fn complete_open_tools(&mut self, context: &HarnessRunContext) -> Vec<HarnessEvent> {
        let tools = std::mem::take(&mut self.tool_names);
        tools
            .into_iter()
            .map(|(part_id, name)| {
                let input = parsed_input(self.tool_inputs.remove(&part_id).as_deref());
                tool_updated(context, &part_id, &name, "completed", input)
            })
            .collect()
    }
}

pub(super) fn text_delta(
    context: &HarnessRunContext,
    part_id: &str,
    field: &str,
    delta: String,
) -> HarnessEvent {
    HarnessEvent::for_context(
        events::MESSAGE_PART_DELTA,
        context,
        json!({
            "messageID": context.message_id,
            "partID": part_id,
            "field": field,
            "delta": delta,
        }),
    )
}

fn part_updated(context: &HarnessRunContext, part_id: &str, mut part: Value) -> HarnessEvent {
    if let Some(part) = part.as_object_mut() {
        part.insert("id".to_owned(), part_id.to_owned().into());
        part.insert("messageID".to_owned(), context.message_id.clone().into());
        part.insert("sessionID".to_owned(), context.session_id.clone().into());
    }
    HarnessEvent::for_context(
        events::MESSAGE_PART_UPDATED,
        context,
        json!({ "part": part }),
    )
}

fn tool_updated(
    context: &HarnessRunContext,
    part_id: &str,
    name: &str,
    status: &str,
    input: Value,
) -> HarnessEvent {
    part_updated(
        context,
        part_id,
        json!({
            "type": "tool",
            "tool": name,
            "state": { "status": status, "input": input },
        }),
    )
}

fn part_id(context: &HarnessRunContext, index: u64, block_type: Option<&str>) -> String {
    if index == 0 && matches!(block_type, None | Some("text")) {
        context.part_id.clone()
    } else {
        format!("{}_part_{index}", context.message_id)
    }
}

fn block_text(block: &Value) -> String {
    block
        .get("text")
        .or_else(|| block.get("thinking"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn delta_text(delta: &Value) -> Option<String> {
    delta
        .get("text")
        .or_else(|| delta.get("thinking"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn parsed_input(raw: Option<&str>) -> Value {
    raw.and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_else(|| json!({}))
}
