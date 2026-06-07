//! Exact-match response cache: hits skip the upstream entirely.

use axum::http::StatusCode;
use litellm_rust::proxy::config::{CacheBackendKind, CacheSettings};
use serde_json::json;
use wiremock::MockServer;

#[path = "cache_support/mod.rs"]
mod support;
use support::{body, body_bytes, build_state, cache_config, cache_config_with, json_mock, send};

#[tokio::test]
async fn serves_identical_request_from_cache() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));
    let b = body();

    let first = body_bytes(send(&state, Some("sk-local"), false, &b).await).await;

    let r2 = send(&state, Some("sk-local"), false, &b).await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-litellm-cache").unwrap(), "hit");
    let second = body_bytes(r2).await;

    assert_eq!(first, second);
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn redb_served_from_cache() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let dir = tempfile::TempDir::new().unwrap();
    let redb_path = dir.path().join("cache.redb");
    let state = build_state(&cache_config_with(
        upstream.uri(),
        Some("sk-local"),
        CacheSettings {
            enabled: true,
            backend: CacheBackendKind::Redb,
            redb_path: Some(redb_path.to_str().unwrap().to_owned()),
            ..Default::default()
        },
    ));
    let b = body();

    let first = body_bytes(send(&state, Some("sk-local"), false, &b).await).await;

    let r2 = send(&state, Some("sk-local"), false, &b).await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-litellm-cache").unwrap(), "hit");
    let second = body_bytes(r2).await;

    assert_eq!(first, second);
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn isolates_cache_by_api_key() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    // No master key configured → any bearer token is accepted, but each is a
    // distinct cache tenant.
    let state = build_state(&cache_config(upstream.uri(), None));
    let b = body();

    let _ = send(&state, Some("tenant-a"), false, &b).await;
    let r2 = send(&state, Some("tenant-b"), false, &b).await;
    // Tenant B must NOT see tenant A's cached response.
    assert!(r2.headers().get("x-litellm-cache").is_none());

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn no_cache_directive_bypasses() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));
    let b = body();

    let _ = send(&state, Some("sk-local"), false, &b).await;
    let _ = send(&state, Some("sk-local"), true, &b).await; // cache-control: no-cache

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn body_cache_no_cache_param_bypasses() {
    // Upstream litellm honours a request-body `cache: {no-cache: true}`; so must we.
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));

    let _ = send(&state, Some("sk-local"), false, &body()).await;
    let mut with_param = body();
    with_param["cache"] = json!({"no-cache": true});
    let _ = send(&state, Some("sk-local"), false, &with_param).await;

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn body_cache_param_stripped_so_no_store_reads_hit() {
    // The proprietary `cache` control field must be stripped before keying, so a
    // request carrying it still hits the entry stored by a plain request (and
    // no-store permits reads) — proving no key fragmentation.
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));

    let _ = send(&state, Some("sk-local"), false, &body()).await;
    let mut with_param = body();
    with_param["cache"] = json!({"no-store": true});
    let r2 = send(&state, Some("sk-local"), false, &with_param).await;
    assert_eq!(r2.headers().get("x-litellm-cache").unwrap(), "hit");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn skips_non_deterministic_requests() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    let state = build_state(&cache_config(upstream.uri(), Some("sk-local")));
    let b = json!({
        "model": "claude",
        "max_tokens": 16,
        "temperature": 0.7,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let _ = send(&state, Some("sk-local"), false, &b).await;
    let _ = send(&state, Some("sk-local"), false, &b).await;

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn unauthenticated_requests_not_cached_when_scoped() {
    let upstream = MockServer::start().await;
    json_mock().mount(&upstream).await;
    // No master key (auth optional) + scope_by_api_key (default on): a request
    // with no API key can't be isolated, so it must not be cached.
    let state = build_state(&cache_config(upstream.uri(), None));
    let b = body();

    let _ = send(&state, None, false, &b).await;
    let r2 = send(&state, None, false, &b).await;
    assert!(r2.headers().get("x-litellm-cache").is_none());

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}
