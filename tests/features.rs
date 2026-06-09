//! Cross-protocol feature mapping: built-in/server tools (#4) and structured
//! outputs (#3). Reasoning/parallel-tool-call mapping lives in
//! `features_reasoning.rs`; message-shape mapping in `features_messages.rs`.

use serde_json::{json, Value};
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[path = "features_support/mod.rs"]
mod support;
use support::{capture_outbound, gemini_ok, model_entry, responses_ok};

// ---- #4 built-in / server tools: dropped cross-protocol, not mangled --------

#[tokio::test]
async fn builtin_web_search_dropped_anthropic_to_chat() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "x",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry("gw", "openai_chat/gpt-4o", &upstream.uri())],
        &upstream,
        "/v1/messages",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": 5}]
        }),
    )
    .await;

    // The built-in must NOT appear as a bogus client function tool.
    let has_web_search = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools.iter().any(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    == Some("web_search")
            })
        })
        .unwrap_or(false);
    assert!(
        !has_web_search,
        "built-in web_search leaked as function tool: {body}"
    );
}

#[tokio::test]
async fn function_tool_preserved_anthropic_to_chat() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "x",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry("gw", "openai_chat/gpt-4o", &upstream.uri())],
        &upstream,
        "/v1/messages",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "get_weather", "input_schema": {"type": "object"}}]
        }),
    )
    .await;

    assert_eq!(body["tools"][0]["function"]["name"], "get_weather");
}

// ---- #3 structured outputs --------------------------------------------------

#[tokio::test]
async fn response_format_chat_to_responses() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(responses_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry("gw", "openai/gpt-5", &upstream.uri())],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "json_schema", "json_schema": {"name": "person", "schema": {"type": "object"}, "strict": true}}
        }),
    )
    .await;

    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert_eq!(body["text"]["format"]["name"], "person");
}

#[tokio::test]
async fn response_format_chat_to_gemini() {
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
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "json_schema", "json_schema": {"name": "person", "schema": {"type": "object", "properties": {"n": {"type": "string"}}}}}
        }),
    )
    .await;

    assert_eq!(
        body["generationConfig"]["responseMimeType"],
        "application/json"
    );
    assert!(body["generationConfig"]["responseJsonSchema"].is_object());
}
