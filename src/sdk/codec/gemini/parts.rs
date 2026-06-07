//! Conversion between Gemini `parts` and IR `ContentBlock`s.

use serde_json::{json, Value};

use super::common::{field, join_text};
use crate::sdk::codec::ir::{ContentBlock, ImageSource};

pub(super) fn part_to_block(part: &Value) -> Option<ContentBlock> {
    let obj = part.as_object()?;
    if let Some(fc) = obj.get("functionCall").or_else(|| obj.get("function_call")) {
        return Some(function_call_block(fc));
    }
    if let Some(fr) = obj
        .get("functionResponse")
        .or_else(|| obj.get("function_response"))
    {
        return Some(function_response_block(fr));
    }
    if let Some(inline) = obj.get("inlineData").or_else(|| obj.get("inline_data")) {
        return inline_data_block(inline);
    }
    // A thought part is text flagged with `thought: true`.
    if obj.get("thought").and_then(Value::as_bool) == Some(true) {
        return Some(ContentBlock::Thinking {
            text: obj
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            signature: None,
        });
    }
    if let Some(text) = obj.get("text").and_then(Value::as_str) {
        return Some(ContentBlock::Text {
            text: text.to_owned(),
        });
    }
    None
}

fn function_call_block(fc: &Value) -> ContentBlock {
    let name = fc.get("name").and_then(Value::as_str).unwrap_or_default();
    ContentBlock::ToolUse {
        id: fc
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_owned(),
        name: name.to_owned(),
        input: fc.get("args").cloned().unwrap_or_else(|| json!({})),
    }
}

fn function_response_block(fr: &Value) -> ContentBlock {
    let name = fr.get("name").and_then(Value::as_str).unwrap_or_default();
    let response = fr.get("response").cloned().unwrap_or(Value::Null);
    let text = match &response {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    ContentBlock::ToolResult {
        tool_use_id: name.to_owned(),
        content: vec![ContentBlock::Text { text }],
        is_error: false,
    }
}

fn inline_data_block(inline: &Value) -> Option<ContentBlock> {
    Some(ContentBlock::Image {
        source: ImageSource::Base64 {
            media_type: field(inline.as_object()?, "mimeType", "mime_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_owned(),
            data: inline
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
    })
}

pub(super) fn block_to_part(block: &ContentBlock) -> Option<Value> {
    match block {
        ContentBlock::Text { text } => Some(json!({"text": text})),
        ContentBlock::Thinking { text, .. } => Some(json!({"text": text, "thought": true})),
        ContentBlock::ToolUse { name, input, .. } => Some(json!({
            "functionCall": {"name": name, "args": normalize_args(input)}
        })),
        ContentBlock::Image { source } => match source {
            ImageSource::Base64 { media_type, data } => {
                Some(json!({"inlineData": {"mimeType": media_type, "data": data}}))
            }
            ImageSource::Url(url) => Some(json!({"fileData": {"fileUri": url}})),
        },
        ContentBlock::ToolResult { .. } => None, // handled at the content level
    }
}

pub(super) fn normalize_args(input: &Value) -> Value {
    match input {
        Value::Object(_) => input.clone(),
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    }
}

pub(super) fn response_object(content: &[ContentBlock]) -> Value {
    let text = join_text(content);
    match serde_json::from_str::<Value>(&text) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({"result": text}),
    }
}
