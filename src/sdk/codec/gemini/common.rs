//! Small shared helpers for the Gemini codec submodules.

use serde_json::{json, Map, Value};

use crate::sdk::codec::ir::{ContentBlock, Usage};

/// Read a field by camelCase or snake_case name (Gemini REST uses camelCase but
/// the proto-JSON form accepts snake_case).
pub(super) fn field<'a>(
    obj: &'a Map<String, Value>,
    camel: &str,
    snake: &str,
) -> Option<&'a Value> {
    obj.get(camel).or_else(|| obj.get(snake))
}

pub(super) fn parse_parts_as_text(v: &Value) -> String {
    let parts = v.get("parts").and_then(Value::as_array);
    let mut text = String::new();
    if let Some(parts) = parts {
        for p in parts {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                text.push_str(t);
            }
        }
    }
    text
}

pub(super) fn join_text(blocks: &[ContentBlock]) -> String {
    let mut text = String::new();
    for block in blocks {
        if let ContentBlock::Text { text: t } = block {
            text.push_str(t);
        }
    }
    text
}

pub(super) fn usage_from_gemini(v: Option<&Value>) -> Usage {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Usage::default();
    };
    // Gemini's `promptTokenCount` is already inclusive of cached tokens.
    let cached = field(obj, "cachedContentTokenCount", "cached_content_token_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        input_tokens: field(obj, "promptTokenCount", "prompt_token_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: field(obj, "candidatesTokenCount", "candidates_token_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}

/// Build a Gemini `usageMetadata` object, adding `cachedContentTokenCount` only
/// on a cache hit (keeps zero-cache output byte-identical).
pub(super) fn gemini_usage(u: &Usage) -> Value {
    let mut usage = json!({
        "promptTokenCount": u.input_tokens,
        "candidatesTokenCount": u.output_tokens,
        "totalTokenCount": u.input_tokens + u.output_tokens,
    });
    if u.cache_read_input_tokens > 0 {
        usage["cachedContentTokenCount"] = json!(u.cache_read_input_tokens);
    }
    usage
}
