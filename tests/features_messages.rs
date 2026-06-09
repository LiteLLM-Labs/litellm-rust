//! Cross-protocol feature mapping: message-shape transforms — consecutive
//! same-role coalescing, multimodal image blocks, and system-message hoisting.

use serde_json::json;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[path = "features_support/mod.rs"]
mod support;
use support::{anthropic_ok, capture_outbound, gemini_ok, model_entry};

// ---- consecutive same-role coalescing (Anthropic/Gemini alternation) --------

/// Parallel tool results from an OpenAI client become separate `role:tool`
/// messages; rendered to Anthropic they must collapse into a single user turn,
/// or Anthropic 400s on consecutive user roles.
#[tokio::test]
async fn parallel_tool_results_coalesced_chat_to_anthropic() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry(
            "gw",
            "anthropic/claude-sonnet-4-5",
            &upstream.uri(),
        )],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "messages": [
                {"role": "user", "content": "weather in SF and NYC?"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_a", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}},
                    {"id": "call_b", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"NYC\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_a", "content": "sunny"},
                {"role": "tool", "tool_call_id": "call_b", "content": "cold"}
            ]
        }),
    )
    .await;

    let messages = body["messages"].as_array().unwrap();
    // user → assistant → user (both tool results merged), no consecutive users.
    assert_eq!(messages.len(), 3, "expected 3 alternating turns: {body}");
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["role"], "user");
    let results = messages[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2, "both tool_results in one turn: {body}");
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "call_a");
    assert_eq!(results[1]["tool_use_id"], "call_b");
    // No two adjacent messages share a role.
    for pair in messages.windows(2) {
        assert_ne!(
            pair[0]["role"], pair[1]["role"],
            "roles must alternate: {body}"
        );
    }
}

/// Same scenario rendered to Gemini: the two functionResponse parts must share a
/// single user content rather than two consecutive user turns.
#[tokio::test]
async fn parallel_tool_results_coalesced_chat_to_gemini() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry("gw", "gemini/gemini-2.5-pro", &upstream.uri())],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_a", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}},
                    {"id": "call_b", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"NYC\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_a", "content": "sunny"},
                {"role": "tool", "tool_call_id": "call_b", "content": "cold"}
            ]
        }),
    )
    .await;

    let contents = body["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 3, "expected 3 alternating contents: {body}");
    assert_eq!(contents[2]["role"], "user");
    let parts = contents[2]["parts"].as_array().unwrap();
    assert_eq!(
        parts.len(),
        2,
        "both functionResponses in one content: {body}"
    );
    assert!(parts[0]["functionResponse"].is_object());
    assert!(parts[1]["functionResponse"].is_object());
}

// ---- multimodal / image content --------------------------------------------

/// An OpenAI `image_url` data URL must render as an Anthropic base64 image block,
/// preserving the media type and base64 payload.
#[tokio::test]
async fn image_chat_to_anthropic() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry(
            "gw",
            "anthropic/claude-sonnet-4-5",
            &upstream.uri(),
        )],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "what is this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}}
            ]}]
        }),
    )
    .await;

    let parts = body["messages"][0]["content"].as_array().unwrap();
    let image = parts
        .iter()
        .find(|b| b["type"] == "image")
        .unwrap_or_else(|| panic!("no image block: {body}"));
    assert_eq!(image["source"]["type"], "base64");
    assert_eq!(image["source"]["media_type"], "image/png");
    assert_eq!(image["source"]["data"], "iVBORw0KGgo=");
}

// ---- system message hoisting -----------------------------------------------

/// An OpenAI `role:"system"` message is hoisted to Anthropic's top-level
/// `system` field (rendered as a text-block array) and must not appear as a chat
/// message role.
#[tokio::test]
async fn system_chat_to_anthropic() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry(
            "gw",
            "anthropic/claude-sonnet-4-5",
            &upstream.uri(),
        )],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "messages": [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"}
            ]
        }),
    )
    .await;

    // Anthropic renders system as an array of text blocks at the top level.
    assert_eq!(body["system"][0]["type"], "text");
    assert_eq!(body["system"][0]["text"], "be terse");
    // No message carries the system role.
    let messages = body["messages"].as_array().unwrap();
    assert!(
        messages.iter().all(|m| m["role"] != "system"),
        "system role leaked into messages: {body}"
    );
}
