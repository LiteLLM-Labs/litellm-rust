//! Cross-protocol conversion: openai_chat inbound ↔ anthropic outbound,
//! exercising the IR pivot with tool calling and streaming.

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

const STREAMING_TOOL_SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"SF\\\"}\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

fn chat_weather_tool_request() -> Value {
    json!({
        "model": "gw-claude",
        "messages": [{"role": "user", "content": "weather in SF?"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
            }
        }]
    })
}

fn chat_text_response() -> Value {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 0,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Hello there"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    })
}

#[tokio::test]
async fn chat_in_anthropic_out_tool_call() {
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
                .uri("/v1/chat/completions")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(chat_weather_tool_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["object"], "chat.completion");
    let tc = &body["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(tc["function"]["name"], "get_weather");
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
    let args: Value = serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["city"], "SF");
}

#[tokio::test]
async fn chat_in_anthropic_out_streaming_tool_call() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(STREAMING_TOOL_SSE.as_bytes(), "text/event-stream"),
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
                .uri("/v1/chat/completions")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "gw-claude",
                        "stream": true,
                        "messages": [{"role": "user", "content": "weather?"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let text = body_text(response).await;
    assert!(text.contains("chat.completion.chunk"), "body: {text}");
    assert!(text.contains("get_weather"));
    assert!(text.contains("\\\"city\\\":"));
    assert!(text.contains("\"finish_reason\":\"tool_calls\""));
    assert!(text.contains("[DONE]"));
}

#[tokio::test]
async fn anthropic_in_chat_out_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_text_response()))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gpt",
        "openai_chat/gpt-4o",
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
                        "model": "gw-gpt",
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
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "Hello there");
    assert_eq!(body["stop_reason"], "end_turn");
    assert_eq!(body["model"], "gw-gpt");
}

#[tokio::test]
async fn anthropic_in_chat_out_streaming_text() {
    let upstream = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse.as_bytes(), "text/event-stream"))
        .mount(&upstream)
        .await;

    let config = config_with(vec![model_entry(
        "gw-gpt",
        "openai_chat/gpt-4o",
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
                        "model": "gw-gpt",
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
    assert!(text.contains("Hello"));
    assert!(text.contains("event: message_stop"));
}
