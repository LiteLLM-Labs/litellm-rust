use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use litellm_rust::{
    http::routes::router,
    proxy::{
        config::{GatewayConfig, GeneralSettings, LiteLlmParams, McpServerEntry, ModelEntry},
        state::AppState,
    },
    sdk::{
        providers::{self, transform::ProviderRegistry},
        router::Router as ModelRouter,
    },
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tower::util::ServiceExt;
use wiremock::{
    matchers::{header as header_match, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn test_config(api_base: String) -> GatewayConfig {
    GatewayConfig {
        model_list: vec![ModelEntry {
            model_name: "claude".to_owned(),
            litellm_params: LiteLlmParams {
                model: "anthropic/claude-sonnet-4-5".to_owned(),
                api_key: Some("sk-ant-test".to_owned()),
                api_base: Some(api_base),
                extra: Default::default(),
            },
        }],
        mcp_servers: Vec::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
        },
    }
}

fn test_config_with_mcp(api_base: String, mcp_url: String) -> GatewayConfig {
    let mut config = test_config(api_base);
    config.mcp_servers = vec![McpServerEntry {
        id: "linear".to_owned(),
        url: mcp_url,
        api_key: Some("mcp-secret".to_owned()),
        headers: Default::default(),
    }];
    config
}

#[tokio::test]
async fn forwards_non_streaming_messages() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header_match("x-api-key", "sk-ant-test"))
        .and(header_match("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .mount(&upstream)
        .await;

    let config = test_config(upstream.uri());
    let app = router(build_state(&config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "claude",
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
}

#[tokio::test]
async fn forwards_streamable_http_mcp_requests() {
    let llm_upstream = MockServer::start().await;
    let mcp_upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header_match("authorization", "Bearer mcp-secret"))
        .and(header_match("mcp-protocol-version", "2025-06-18"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [] }
        })))
        .mount(&mcp_upstream)
        .await;

    let config = test_config_with_mcp(llm_upstream.uri(), format!("{}/mcp", mcp_upstream.uri()));
    let app = router(build_state(&config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/linear")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .header("mcp-protocol-version", "2025-06-18")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/list",
                        "params": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("\"tools\""));
}

#[tokio::test]
async fn rejects_mcp_without_master_key() {
    let llm_upstream = MockServer::start().await;
    let mcp_upstream = MockServer::start().await;
    let config = test_config_with_mcp(llm_upstream.uri(), format!("{}/mcp", mcp_upstream.uri()));
    let app = router(build_state(&config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/linear")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/list",
                        "params": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

fn build_router(config: &GatewayConfig) -> ModelRouter {
    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);
    ModelRouter::from_config(config, &providers).unwrap()
}

fn build_state(config: &GatewayConfig) -> Arc<AppState> {
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config.clone(), build_router(config), http, HashMap::new()).unwrap())
}

#[tokio::test]
async fn serves_lite_harness_ui_and_compatibility_routes() {
    let ui_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(ui_dir.path().join("sessions")).unwrap();
    fs::create_dir_all(ui_dir.path().join("_next/static/chunks")).unwrap();
    fs::write(
        ui_dir.path().join("sessions/index.html"),
        "<html>sessions</html>",
    )
    .unwrap();
    fs::write(ui_dir.path().join("404.html"), "<html>not found</html>").unwrap();
    fs::write(
        ui_dir.path().join("_next/static/chunks/app.js"),
        "console.log('ok');",
    )
    .unwrap();
    std::env::set_var("LITELLM_UI_DIR", ui_dir.path());

    let upstream = MockServer::start().await;
    let config = test_config(upstream.uri());
    let app = router(build_state(&config));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/sessions/"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/sessions/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("sessions"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("claude"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_litellm/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/whoami")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn rejects_missing_master_key() {
    let upstream = MockServer::start().await;
    let config = test_config(upstream.uri());
    let app = router(build_state(&config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "claude",
                        "max_tokens": 16,
                        "messages": [{"role": "user", "content": "hi"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn forwards_streaming_messages_as_sse() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("event: message_start\ndata: {\"type\":\"message_start\"}\n\n"),
        )
        .mount(&upstream)
        .await;

    let config = test_config(upstream.uri());
    let app = router(build_state(&config));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "claude",
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
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body)
        .unwrap()
        .contains("message_start"));
}
