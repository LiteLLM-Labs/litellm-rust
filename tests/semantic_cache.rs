//! Semantic response cache (feature `semantic-cache`). Only built/run with
//! `cargo test --features semantic-cache`.
#![cfg(feature = "semantic-cache")]

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
};
use litellm_rust::{
    http::routes::router,
    proxy::{
        config::{
            CacheSettings, GatewayConfig, GeneralSettings, LiteLlmParams, ModelEntry,
            SemanticCacheSettings,
        },
        state::AppState,
    },
    sdk::{
        providers::{self, transform::ProviderRegistry},
        router::Router as ModelRouter,
    },
};
use serde_json::json;
use tower::util::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn build_state(config: &GatewayConfig) -> Arc<AppState> {
    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);
    let model_router = ModelRouter::from_config(config, &providers).unwrap();
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config.clone(), model_router, http, HashMap::new(), None).unwrap())
}

/// A gateway config with exact-match cache OFF and semantic cache ON, pointing
/// both chat and embedding upstreams at `server_uri`.
fn semantic_config(server_uri: String) -> GatewayConfig {
    GatewayConfig {
        model_list: vec![ModelEntry {
            model_name: "claude".to_owned(),
            litellm_params: LiteLlmParams {
                model: "anthropic/claude-sonnet-4-5".to_owned(),
                api_key: Some("sk-ant-test".to_owned()),
                api_base: Some(server_uri.clone()),
                wire_api: None,
                extra: Default::default(),
            },
        }],
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            cache: CacheSettings {
                enabled: false, // exact-match off: only semantic is exercised
                semantic: SemanticCacheSettings {
                    enabled: true,
                    embedding_api_base: Some(server_uri),
                    similarity_threshold: 0.5,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
        agents: Vec::new(),
    }
}

async fn send(state: &Arc<AppState>, text: &str) -> Response {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(header::AUTHORIZATION, "Bearer sk-local")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "model": "claude",
                "max_tokens": 16,
                "messages": [{"role": "user", "content": text}]
            })
            .to_string(),
        ))
        .unwrap();
    router(state.clone()).oneshot(req).await.unwrap()
}

#[tokio::test]
async fn near_match_served_from_semantic_cache() {
    let server = MockServer::start().await;
    // Embeddings: same vector for any input → every query is a "match".
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"embedding": [1.0, 0.0, 0.0]}]
        })))
        .mount(&server)
        .await;
    // Chat upstream.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "four"}],
            "usage": {"input_tokens": 3, "output_tokens": 1}
        })))
        .mount(&server)
        .await;

    let config = semantic_config(server.uri());
    let state = build_state(&config);

    // Different wording, but the mock embeds both to the same vector.
    let first = to_bytes(send(&state, "what is 2 + 2").await.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();

    let r2 = send(&state, "compute two plus two please").await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-litellm-cache").unwrap(), "semantic");
    let second = to_bytes(r2.into_body(), 1 << 20).await.unwrap().to_vec();
    assert_eq!(first, second);

    // The chat upstream was only called once; the second answer came from cache.
    let chat_calls = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/messages")
        .count();
    assert_eq!(chat_calls, 1);
}

#[tokio::test]
async fn embedding_failure_falls_open() {
    let server = MockServer::start().await;
    // Embeddings endpoint errors → semantic cache must fall open (call upstream).
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .mount(&server)
        .await;

    let config = semantic_config(server.uri());
    let state = build_state(&config);

    let r1 = send(&state, "hello").await;
    assert_eq!(r1.status(), StatusCode::OK);
    let r2 = send(&state, "hello").await;
    assert_eq!(r2.status(), StatusCode::OK);
    // Both must reach the upstream (no false cache hit on embedding failure).
    let _ = to_bytes(r2.into_body(), 1 << 20).await.unwrap();

    let chat_calls = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/messages")
        .count();
    assert_eq!(chat_calls, 2);
}
