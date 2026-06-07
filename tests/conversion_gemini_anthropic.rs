//! Cross-protocol conversion: gemini inbound ↔ anthropic outbound.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use litellm_rust::http::routes::router;
use serde_json::{json, Value};
use tower::util::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[path = "conversion_support/mod.rs"]
mod support;
use support::*;

fn gemini_weather_tool_request() -> Value {
    json!({
        "contents": [{"role": "user", "parts": [{"text": "weather in SF?"}]}],
        "tools": [{"functionDeclarations": [{
            "name": "get_weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
        }]}]
    })
}

#[tokio::test]
async fn gemini_in_anthropic_out_tool_call() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_tool_use_body()))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gemini",
        "anthropic/claude-sonnet-4-5",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gw-gemini:generateContent")
                .header("x-goog-api-key", "sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(gemini_weather_tool_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    let fc = &body["candidates"][0]["content"]["parts"][0]["functionCall"];
    assert_eq!(fc["name"], "get_weather");
    assert_eq!(fc["args"]["city"], "SF");
}

#[tokio::test]
async fn anthropic_in_gemini_out_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {"role": "model", "parts": [{"text": "Hi there"}]},
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 2, "totalTokenCount": 5}
        })))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gem",
        "gemini/gemini-2.5-pro",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "gw-gem",
                        "max_tokens": 16,
                        "messages": [{"role": "user", "content": "hi"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "Hi there");
    assert_eq!(body["stop_reason"], "end_turn");
}

#[tokio::test]
async fn gemini_in_anthropic_out_streaming_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(ANTHROPIC_TEXT_SSE.as_bytes(), "text/event-stream"),
        )
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gemini",
        "anthropic/claude-sonnet-4-5",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gw-gemini:streamGenerateContent?alt=sse")
                .header("x-goog-api-key", "sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("candidates"), "body: {text}");
    assert!(text.contains("Hello"));
    assert!(text.contains("finishReason"));
}

#[tokio::test]
async fn anthropic_in_gemini_out_streaming_text() {
    let upstream = MockServer::start().await;
    let sse = concat!(
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hel\"}]},\"index\":0}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"lo\"}]},\"index\":0}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[]},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2,\"totalTokenCount\":5}}\n\n",
    );
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse.as_bytes(), "text/event-stream"))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gem",
        "gemini/gemini-2.5-pro",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "gw-gem",
                        "max_tokens": 16,
                        "stream": true,
                        "messages": [{"role": "user", "content": "hi"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("event: message_start"), "body: {text}");
    assert!(text.contains("content_block_delta"));
    assert!(text.contains("Hel"));
    assert!(text.contains("lo"));
    assert!(text.contains("event: message_stop"));
}
