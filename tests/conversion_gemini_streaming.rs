//! Gemini outbound streaming regressions: tool-call stop reason and the
//! thinking-then-text content-block split.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use litellm_rust::http::routes::router;
use serde_json::json;
use tower::util::ServiceExt;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[path = "conversion_support/mod.rs"]
mod support;
use support::*;

const TOOL_CALL_STOP_SSE: &str = concat!(
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"get_weather\",\"args\":{\"city\":\"SF\"}}}]},\"index\":0}]}\n\n",
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[]},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2,\"totalTokenCount\":5}}\n\n",
);

const THINKING_THEN_TEXT_SSE: &str = concat!(
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"pondering\",\"thought\":true}]},\"index\":0}]}\n\n",
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Answer\"}]},\"index\":0}]}\n\n",
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[]},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2,\"totalTokenCount\":5}}\n\n",
);

/// Gemini reports finishReason STOP even when it streamed a functionCall. The
/// stop reason must still surface as a tool call to the client (regression).
#[tokio::test]
async fn chat_in_gemini_out_streaming_tool_call_stop_reason() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TOOL_CALL_STOP_SSE),
        )
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
                .uri("/v1/chat/completions")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "gw-gem",
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
    let text = body_text(response).await;
    assert!(text.contains("get_weather"), "body: {text}");
    assert!(
        text.contains("\"finish_reason\":\"tool_calls\""),
        "stop reason must be tool_calls, not stop: {text}"
    );
    assert!(text.contains("[DONE]"));
}

/// Gemini streams `thought` parts before the answer text. The two must land on
/// separate content blocks (thinking then text), not share one index, or the
/// answer text gets mislabelled as thinking (regression).
#[tokio::test]
async fn anthropic_in_gemini_out_streaming_thinking_then_text() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(THINKING_THEN_TEXT_SSE),
        )
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
    // A thinking block carrying the thought, and a *separate* text block for the
    // answer — proving the answer wasn't merged onto the thinking index. (serde
    // serializes object keys alphabetically, hence text-before-type below.)
    assert!(text.contains("thinking_delta"), "body: {text}");
    assert!(text.contains("pondering"), "body: {text}");
    assert!(
        text.contains("\"text\":\"Answer\",\"type\":\"text_delta\""),
        "answer must be a text_delta, not thinking: {text}"
    );
    assert!(
        text.contains("{\"text\":\"\",\"type\":\"text\"}"),
        "a dedicated text content_block_start must open: {text}"
    );
}
