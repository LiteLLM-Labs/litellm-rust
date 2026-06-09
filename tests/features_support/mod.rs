//! Shared fixtures for the cross-protocol feature-mapping integration tests.
//! Uniquely named (`features_support`) to avoid colliding with other agents'
//! tests/ modules. Each test posts an inbound request and asserts the shape of
//! the OUTBOUND (upstream) request the gateway produces.
#![allow(dead_code)]

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use litellm_rust::{
    http::routes::router,
    proxy::{
        config::{GatewayConfig, GeneralSettings, LiteLlmParams, ModelEntry},
        state::AppState,
    },
    sdk::{
        providers::{self, transform::ProviderRegistry},
        router::Router as ModelRouter,
    },
};
use serde_json::{json, Value};
use tower::util::ServiceExt;
use wiremock::MockServer;

pub fn model_entry(model_name: &str, model: &str, api_base: &str) -> ModelEntry {
    ModelEntry {
        model_name: model_name.to_owned(),
        litellm_params: LiteLlmParams {
            model: model.to_owned(),
            api_key: Some("sk-upstream".to_owned()),
            api_base: Some(api_base.to_owned()),
            wire_api: None,
            extra: Default::default(),
        },
    }
}

pub fn build_state(entries: Vec<ModelEntry>) -> Arc<AppState> {
    let config = GatewayConfig {
        model_list: entries,
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            ..Default::default()
        },
        agents: Vec::new(),
    };
    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);
    let model_router = ModelRouter::from_config(&config, &providers).unwrap();
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config, model_router, http, HashMap::new(), None).unwrap())
}

pub fn anthropic_ok() -> Value {
    json!({
        "id": "m", "type": "message", "role": "assistant", "model": "x",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "usage": {"input_tokens": 1, "output_tokens": 1}
    })
}

pub fn responses_ok() -> Value {
    json!({
        "id": "r", "object": "response", "model": "x", "status": "completed",
        "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
}

pub fn gemini_ok() -> Value {
    json!({
        "candidates": [{"content": {"role": "model", "parts": [{"text": "ok"}]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1}
    })
}

/// POST an inbound request and return the body the gateway sent upstream.
pub async fn capture_outbound(
    entries: Vec<ModelEntry>,
    upstream: &MockServer,
    uri: &str,
    inbound_body: Value,
) -> Value {
    let app = router(build_state(entries));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(inbound_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let reqs = upstream.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1, "expected exactly one upstream request");
    serde_json::from_slice(&reqs[0].body).unwrap()
}
