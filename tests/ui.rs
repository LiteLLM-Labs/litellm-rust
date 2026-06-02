use std::{collections::HashMap, fs, sync::Arc};

use axum::{
    body::{to_bytes, Body},
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
use tempfile::TempDir;
use tower::util::ServiceExt;
use wiremock::MockServer;

#[tokio::test]
async fn serves_lite_harness_ui_and_compatibility_routes() {
    let ui_dir = write_ui_fixture();
    std::env::set_var("LITELLM_UI_DIR", ui_dir.path());

    let upstream = MockServer::start().await;
    let app = router(build_state(&test_config(upstream.uri())));

    assert_redirects_to_sessions(app.clone()).await;
    assert_serves_sessions_html(app.clone()).await;
    assert_lists_gateway_models(app.clone()).await;
    assert_serves_gateway_health(app.clone()).await;
    assert_accepts_whoami_master_key(app).await;
}

fn write_ui_fixture() -> TempDir {
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
    ui_dir
}

async fn assert_redirects_to_sessions(app: axum::Router) {
    let response = get(app, "/").await;
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/sessions/"
    );
}

async fn assert_serves_sessions_html(app: axum::Router) {
    let response = get(app, "/sessions/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_body_contains(response, "sessions").await;
}

async fn assert_lists_gateway_models(app: axum::Router) {
    let response = get(app, "/v1/models").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_body_contains(response, "claude").await;
}

async fn assert_serves_gateway_health(app: axum::Router) {
    assert_eq!(get(app, "/_litellm/health").await.status(), StatusCode::OK);
}

async fn assert_accepts_whoami_master_key(app: axum::Router) {
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

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn assert_body_contains(response: axum::response::Response, needle: &str) {
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains(needle));
}

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

fn build_router(config: &GatewayConfig) -> ModelRouter {
    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);
    ModelRouter::from_config(config, &providers).unwrap()
}

fn build_state(config: &GatewayConfig) -> Arc<AppState> {
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config.clone(), build_router(config), http, HashMap::new()).unwrap())
}
