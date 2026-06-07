//! Cross-protocol feature mapping: reasoning effort/budget (#5) and
//! parallel-tool-call control (#2), including the accepted degradation when
//! `parallel_tool_calls:false` arrives without a `tool_choice` carrier (#1).

use serde_json::json;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[path = "features_support/mod.rs"]
mod support;
use support::{anthropic_ok, capture_outbound, gemini_ok, model_entry};

// ---- #5 reasoning effort / budget ------------------------------------------

#[tokio::test]
async fn reasoning_chat_to_anthropic_clamps_and_drops_temperature() {
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
            "max_tokens": 20000,
            "temperature": 0.5,
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;

    assert_eq!(body["thinking"]["type"], "enabled");
    let budget = body["thinking"]["budget_tokens"].as_u64().unwrap();
    assert!(
        (1024..20000).contains(&budget),
        "budget out of range: {budget}"
    );
    // Extended thinking forbids a custom temperature.
    assert!(
        body.get("temperature").is_none(),
        "temperature must be dropped: {body}"
    );
}

#[tokio::test]
async fn reasoning_chat_to_gemini_budget() {
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
            "reasoning_effort": "medium",
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;

    assert!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"]
            .as_u64()
            .unwrap()
            > 0
    );
}

// ---- #2 parallel tool calls -------------------------------------------------

#[tokio::test]
async fn parallel_disabled_chat_to_anthropic() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok()))
        .mount(&upstream)
        .await;

    let body = capture_outbound(
        vec![model_entry("gw", "anthropic/claude-sonnet-4-5", &upstream.uri())],
        &upstream,
        "/v1/chat/completions",
        json!({
            "model": "gw",
            "max_tokens": 16,
            "tool_choice": "required",
            "parallel_tool_calls": false,
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}],
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;

    assert_eq!(body["tool_choice"]["type"], "any");
    assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
}

#[tokio::test]
async fn parallel_disabled_anthropic_to_chat() {
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
            "tool_choice": {"type": "any", "disable_parallel_tool_use": true},
            "tools": [{"name": "f", "input_schema": {"type": "object"}}],
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;

    assert_eq!(body["parallel_tool_calls"], false);
}

// ---- parallel_tool_calls without tool_choice (accepted degradation #1) ------

/// Anthropic only expresses "disable parallel tool use" as a field nested inside
/// `tool_choice`. With `parallel_tool_calls:false` but no `tool_choice`, there is
/// no carrier object, so the intent is dropped. This is an accepted semantic
/// degradation (option #1): rather than synthesize a `tool_choice`, we let the
/// upstream default (parallel allowed) stand.
#[tokio::test]
async fn parallel_disabled_without_tool_choice_chat_to_anthropic() {
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
            "parallel_tool_calls": false,
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}],
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;

    assert!(
        !body.to_string().contains("disable_parallel_tool_use"),
        "intent must be dropped without tool_choice carrier: {body}"
    );
}
