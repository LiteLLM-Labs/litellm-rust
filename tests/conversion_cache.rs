//! Auto-injected prompt-cache breakpoints (openai_chat inbound → anthropic).

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use litellm_rust::{http::routes::router, proxy::config::PromptCachingSettings};
use serde_json::{json, Value};
use tower::util::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[path = "conversion_support/mod.rs"]
mod support;
use support::*;

fn cached_text_response() -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 5,
            "output_tokens": 2,
            "cache_creation_input_tokens": 100,
            "cache_read_input_tokens": 0
        }
    })
}

fn cache_chat_request() -> Value {
    json!({
        "model": "gw-claude",
        "messages": [
            {"role": "system", "content": "you are a careful assistant"},
            {"role": "user", "content": "hello there"}
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object"}
            }
        }]
    })
}

#[tokio::test]
async fn auto_injects_anthropic_cache_breakpoints() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cached_text_response()))
        .mount(&upstream)
        .await;

    let mut config = config_with(vec![model_entry(
        "gw-claude",
        "anthropic/claude-sonnet-4-5",
        &upstream.uri(),
    )]);
    config.general_settings.prompt_caching = PromptCachingSettings {
        enabled: true,
        auto_inject: true,
        max_breakpoints: 4,
        min_tokens: 1,
        chars_per_token: 1,
    };
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(cache_chat_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // The gateway must have placed cache_control on the stable prefix it sent up.
    let reqs = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let tools = sent["tools"].as_array().unwrap();
    assert!(
        tools.last().unwrap().get("cache_control").is_some(),
        "expected tools breakpoint, sent: {sent}"
    );
    let system = sent["system"].as_array().unwrap();
    assert!(
        system.last().unwrap().get("cache_control").is_some(),
        "expected system breakpoint, sent: {sent}"
    );

    // And the OpenAI client sees inclusive prompt_tokens (5 + 100 created).
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["usage"]["prompt_tokens"], 105);
}
