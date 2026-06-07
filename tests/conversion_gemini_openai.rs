//! Cross-protocol conversion: gemini inbound ↔ openai_chat / openai_responses,
//! plus openai_responses / gemini outbound text.

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

fn chat_tool_call_response() -> Value {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 0,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
}

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
async fn gemini_in_chat_out_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi there"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        })))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw",
        "openai_chat/gpt-4o",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gw:generateContent")
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
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "Hi there"
    );
    assert!(body["candidates"][0]["finishReason"].is_string());
}

#[tokio::test]
async fn gemini_in_chat_out_tool_call() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_tool_call_response()))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw",
        "openai_chat/gpt-4o",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gw:generateContent")
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
async fn responses_in_gemini_out_text() {
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
        "gw",
        "gemini/gemini-2.5-pro",
        &upstream.uri(),
    )]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"model": "gw", "input": "hi"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["output"][0]["content"][0]["text"], "Hi there");
}

#[tokio::test]
async fn gemini_in_responses_out_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_1",
            "object": "response",
            "model": "gpt-5",
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hi there"}]
            }],
            "usage": {"input_tokens": 3, "output_tokens": 2}
        })))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry("gw", "openai/gpt-5", &upstream.uri())]);
    let app = router(build_state(config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gw:generateContent")
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
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "Hi there"
    );
}
