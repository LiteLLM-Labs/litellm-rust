//! Non-streaming response decoding for the Responses codec.

use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::codec::ir::{ChatResponse, ContentBlock, StopReason},
};

use super::parse::{function_call_to_block, usage_from_responses};

pub(super) fn parse_response(body: Value) -> Result<ChatResponse, GatewayError> {
    let obj = body.as_object().ok_or_else(|| {
        GatewayError::InvalidJsonMessage("response body must be a JSON object".to_owned())
    })?;

    let mut content = Vec::new();
    let mut saw_tool = false;
    if let Some(output) = obj.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            if let Some(text) = part
                                .get("text")
                                .and_then(Value::as_str)
                                .filter(|t| !t.is_empty())
                            {
                                content.push(ContentBlock::Text {
                                    text: text.to_owned(),
                                });
                            }
                        }
                    }
                }
                Some("function_call") => {
                    saw_tool = true;
                    content.push(function_call_to_block(item));
                }
                Some("reasoning") => {
                    if let Some(text) = reasoning_text(item) {
                        content.push(ContentBlock::Thinking {
                            text,
                            signature: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let stop_reason = decode_stop_reason(obj, saw_tool);

    Ok(ChatResponse {
        id: obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        model: obj
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content,
        stop_reason,
        usage: usage_from_responses(obj.get("usage")),
    })
}

/// Map a Responses object `status` to an IR stop reason. A `failed` result is an
/// HTTP-200 body carrying an error, so it must surface as an error stop reason
/// rather than a clean end turn (mirrors the streaming `response.failed` path).
fn decode_stop_reason(obj: &serde_json::Map<String, Value>, saw_tool: bool) -> Option<StopReason> {
    match obj.get("status").and_then(Value::as_str) {
        Some("incomplete") => Some(StopReason::MaxTokens),
        Some("failed") => {
            let message = obj
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("response failed");
            Some(StopReason::Other(format!("error: {message}")))
        }
        _ if saw_tool => Some(StopReason::ToolUse),
        _ => Some(StopReason::EndTurn),
    }
}

fn reasoning_text(item: &Value) -> Option<String> {
    let summary = item.get("summary").and_then(Value::as_array)?;
    let mut text = String::new();
    for part in summary {
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            text.push_str(t);
        }
    }
    (!text.is_empty()).then_some(text)
}
