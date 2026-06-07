//! Cross-protocol conversion: openai_responses inbound ↔ anthropic outbound.

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

fn responses_weather_tool_request() -> Value {
    json!({
        "model": "gw-claude",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "weather in SF?"}]}],
        "tools": [{
            "type": "function",
            "name": "get_weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
        }]
    })
}

#[tokio::test]
async fn responses_in_anthropic_out_tool_call() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_tool_use_body()))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-claude",
        "anthropic/claude-sonnet-4-5",
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
                .body(Body::from(responses_weather_tool_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["object"], "response");
    let fc = &body["output"][0];
    assert_eq!(fc["type"], "function_call");
    assert_eq!(fc["name"], "get_weather");
    let args: Value = serde_json::from_str(fc["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["city"], "SF");
}

#[tokio::test]
async fn anthropic_in_responses_out_text() {
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

    let config = config_with(vec![model_entry("gw-oai", "openai/gpt-5", &upstream.uri())]);
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
                        "model": "gw-oai",
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
async fn responses_in_anthropic_out_streaming_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_TEXT_SSE),
        )
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-claude",
        "anthropic/claude-sonnet-4-5",
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
                    json!({"model": "gw-claude", "stream": true, "input": "hi"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("response.output_text.delta"), "body: {text}");
    assert!(text.contains("Hello"));
    assert!(text.contains("response.completed"));
}

#[tokio::test]
async fn anthropic_in_responses_out_streaming_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(RESPONSES_TEXT_SSE),
        )
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry("gw-oai", "openai/gpt-5", &upstream.uri())]);
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
                        "model": "gw-oai",
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
    assert!(text.contains("Hello"));
    assert!(text.contains("event: message_stop"));
}
