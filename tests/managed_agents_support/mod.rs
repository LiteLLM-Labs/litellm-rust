use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use litellm_rust::{
    db::managed_agents::pool as managed_agents_pool,
    http::routes::router,
    proxy::{
        config::{GatewayConfig, GeneralSettings},
        state::AppState,
    },
    sdk::{providers::transform::ProviderRegistry, router::Router as ModelRouter},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::util::ServiceExt;

pub mod flows;

pub struct AppFixture {
    pub app: axum::Router,
    pool: PgPool,
}

impl AppFixture {
    pub async fn new() -> Option<Self> {
        let database_url = std::env::var("TEST_DATABASE_URL").ok()?;
        let pool = managed_agents_pool::connect(&database_url).await.unwrap();
        managed_agents_pool::migrate(&pool).await.unwrap();
        reset_tables(&pool).await;
        Some(Self {
            app: router(build_state(pool.clone())),
            pool,
        })
    }
}

fn build_state(pool: PgPool) -> Arc<AppState> {
    let config = GatewayConfig {
        model_list: Vec::new(),
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            database_url: Some("postgres://test".to_owned()),
            ..Default::default()
        },
        agents: Vec::new(),
    };
    let http = AppState::build_http_client().unwrap();
    Arc::new(AppState::new(config, empty_router(), http, HashMap::new(), Some(pool)).unwrap())
}

fn empty_router() -> ModelRouter {
    ModelRouter::from_config(
        &GatewayConfig {
            model_list: Vec::new(),
            mcp_servers: HashMap::new(),
            general_settings: GeneralSettings::default(),
            agents: Vec::new(),
        },
        &ProviderRegistry::new(),
    )
    .unwrap()
}

pub async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> Value {
    let response = request(
        app,
        method,
        uri,
        body.map(|value| value.to_string()),
        "application/json",
    )
    .await;
    assert!(
        response.status().is_success(),
        "{} {} returned {}",
        method,
        uri,
        response.status()
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap_or_else(|_| json!({}))
}

pub async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
    content_type: &str,
    expected: StatusCode,
) -> String {
    let response = request(app, method, uri, body, content_type).await;
    assert_eq!(response.status(), expected);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
    content_type: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer sk-local")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body.unwrap_or_default()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn reset_tables(pool: &PgPool) {
    sqlx::query(
        r#"
        TRUNCATE
          "LiteLLM_ManagedAgentInboxItemsTable",
          "LiteLLM_ManagedAgentRunsTable",
          "LiteLLM_ManagedAgentFilesTable",
          "LiteLLM_ManagedAgentMemoriesTable",
          "LiteLLM_ManagedAgentsTable",
          "LiteLLM_ManagedAgentSessionsTable",
          "LiteLLM_ManagedAgentSkillsTable",
          "LiteLLM_SavedAgentsTable"
        CASCADE
        "#,
    )
    .execute(pool)
    .await
    .unwrap();
}
