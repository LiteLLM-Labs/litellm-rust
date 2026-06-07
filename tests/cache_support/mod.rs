//! Shared fixtures for the exact-match cache integration tests. Uniquely named
//! (`cache_support`) so it does not collide with other agents' tests/ modules.
#![allow(dead_code)] // each consumer file uses a subset of these helpers

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
    response::Response,
};
use litellm_rust::{
    http::routes::router,
    proxy::{
        config::{CacheSettings, GatewayConfig, GeneralSettings, LiteLlmParams, ModelEntry},
        state::AppState,
    },
    sdk::{
        providers::{self, transform::ProviderRegistry},
        router::Router as ModelRouter,
    },
};
use serde_json::{json, Value};
use tower::util::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, ResponseTemplate,
};

pub fn cache_config(api_base: String, master_key: Option<&str>) -> GatewayConfig {
    cache_config_with(
        api_base,
        master_key,
        CacheSettings {
            enabled: true,
            ..Default::default()
        },
    )
}

pub fn cache_config_with(
    api_base: String,
    master_key: Option<&str>,
    cache: CacheSettings,
) -> GatewayConfig {
    GatewayConfig {
        model_list: vec![ModelEntry {
            model_name: "claude".to_owned(),
            litellm_params: LiteLlmParams {
                model: "anthropic/claude-sonnet-4-5".to_owned(),
                api_key: Some("sk-ant-test".to_owned()),
                api_base: Some(api_base),
                wire_api: None,
                extra: Default::default(),
            },
        }],
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: master_key.map(str::to_owned),
            cache,
            ..Default::default()
        },
        agents: Vec::new(),
    }
}

pub fn build_state(config: &GatewayConfig) -> Arc<AppState> {
    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);
    let model_router = ModelRouter::from_config(config, &providers).unwrap();
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config.clone(), model_router, http, HashMap::new(), None).unwrap())
}

pub async fn send(
    state: &Arc<AppState>,
    auth: Option<&str>,
    no_cache: bool,
    body: &Value,
) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(a) = auth {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {a}"));
    }
    if no_cache {
        builder = builder.header("cache-control", "no-cache");
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();
    router(state.clone()).oneshot(req).await.unwrap()
}

pub async fn body_bytes(resp: Response) -> Vec<u8> {
    to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()
}

pub fn json_mock() -> Mock {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
}

pub fn body() -> Value {
    // temperature: 0 makes the request deterministic, hence exact-cacheable.
    json!({
        "model": "claude",
        "max_tokens": 16,
        "temperature": 0,
        "messages": [{"role": "user", "content": "hi"}]
    })
}
