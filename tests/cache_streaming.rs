//! Streaming-response caching: SSE replay on hit, and the size cap that keeps
//! oversized streams out of the store.

use std::time::Duration;

use axum::http::{header, StatusCode};
use litellm_rust::proxy::config::CacheSettings;
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[path = "cache_support/mod.rs"]
mod support;
use support::{body_bytes, build_state, cache_config, cache_config_with, send};

#[tokio::test]
async fn oversized_stream_not_cached() {
    let upstream = MockServer::start().await;
    let sse = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse.as_bytes(), "text/event-stream"))
        .mount(&upstream)
        .await;
    let state = build_state(&cache_config_with(
        upstream.uri(),
        Some("sk-local"),
        CacheSettings {
            enabled: true,
            max_stream_bytes: 8, // far below the SSE body → buffering aborts
            ..Default::default()
        },
    ));
    let b = json!({
        "model": "claude",
        "max_tokens": 16,
        "temperature": 0,
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let _ = body_bytes(send(&state, Some("sk-local"), false, &b).await).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r2 = send(&state, Some("sk-local"), false, &b).await;
    // Over-cap stream was forwarded but not stored → second request hits upstream.
    assert!(r2.headers().get("x-litellm-cache").is_none());

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn caches_and_replays_streaming() {
    let upstream = MockServer::start().await;
    let sse = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse.as_bytes(), "text/event-stream"))
        .mount(&upstream)
        .await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));
    let b = json!({
        "model": "claude",
        "max_tokens": 16,
        "temperature": 0,
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let first = body_bytes(send(&state, Some("sk-local"), false, &b).await).await;
    // The streaming store is spawned on clean completion; give it a moment.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let r2 = send(&state, Some("sk-local"), false, &b).await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-litellm-cache").unwrap(), "hit");
    assert_eq!(
        r2.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let second = body_bytes(r2).await;

    assert_eq!(first, second);
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}
