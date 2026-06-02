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

#[tokio::test]
async fn managed_agent_endpoints_round_trip_against_postgres() {
    let Some(database_url) = std::env::var("TEST_DATABASE_URL").ok() else {
        eprintln!("skipping managed agent integration test: TEST_DATABASE_URL is not set");
        return;
    };

    let pool = managed_agents_pool::connect(&database_url).await.unwrap();
    managed_agents_pool::migrate(&pool).await.unwrap();
    reset_tables(&pool).await;

    let app = router(build_state(pool.clone()));

    let created = request_json(
        app.clone(),
        "POST",
        "/api/agents",
        Some(json!({
            "name": "ops-agent",
            "owner_id": "user-1",
            "prompt": "watch deploys"
        })),
    )
    .await;
    let agent_id = created["id"].as_str().unwrap().to_owned();

    let listed = request_json(app.clone(), "GET", "/api/agents?owner_id=user-1", None).await;
    assert_eq!(listed["agents"].as_array().unwrap().len(), 1);

    let paused = request_json(
        app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/pause"),
        None,
    )
    .await;
    assert_eq!(paused["status"], "paused");

    let resumed = request_json(
        app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/resume"),
        None,
    )
    .await;
    assert_eq!(resumed["status"], "active");

    let memory = request_json(
        app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/memory"),
        Some(json!({"key": "deploys", "value": "watch prod", "always_on": true})),
    )
    .await;
    assert_eq!(memory["key"], "deploys");

    let memories = request_json(
        app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/memory"),
        None,
    )
    .await;
    assert_eq!(memories["memories"].as_array().unwrap().len(), 1);

    request_raw(
        app.clone(),
        "PUT",
        &format!("/api/agents/{agent_id}/files/notes.txt"),
        Some("hello".to_owned()),
        "text/plain",
        StatusCode::OK,
    )
    .await;

    let files = request_json(
        app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/files"),
        None,
    )
    .await;
    assert_eq!(files["files"].as_array().unwrap().len(), 1);

    let file = request_raw(
        app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/files/notes.txt"),
        None,
        "application/json",
        StatusCode::OK,
    )
    .await;
    assert_eq!(file, "hello");

    let run = request_json(
        app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/run"),
        Some(json!({})),
    )
    .await;
    let run_id = run["run_id"].as_str().unwrap().to_owned();
    assert!(run["logs_url"]
        .as_str()
        .unwrap()
        .contains(&format!("/api/agents/{agent_id}/runs/{run_id}/logs")));

    let runs = request_json(
        app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/runs"),
        None,
    )
    .await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 1);

    request_raw(
        app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/runs/{run_id}/logs"),
        None,
        "application/json",
        StatusCode::OK,
    )
    .await;

    request_raw(
        app.clone(),
        "PUT",
        &format!("/api/agents/{agent_id}/files/bad.xlsx"),
        Some(json!({"content_base64": "not base64 !!!"}).to_string()),
        "application/json",
        StatusCode::BAD_REQUEST,
    )
    .await;

    let skill = request_json(
        app.clone(),
        "POST",
        "/api/skills",
        Some(json!({"name": "triage", "content": "do triage", "owner_id": "user-1"})),
    )
    .await;
    let skill_id = skill["id"].as_str().unwrap();
    let skill = request_json(
        app.clone(),
        "PATCH",
        &format!("/api/skills/{skill_id}"),
        Some(json!({"description": "daily"})),
    )
    .await;
    assert_eq!(skill["description"], "daily");

    seed_inbox(&pool).await;
    let inbox = request_json(app.clone(), "GET", "/api/inbox?filter=attention", None).await;
    assert_eq!(inbox["items"].as_array().unwrap().len(), 2);

    request_json(
        app.clone(),
        "POST",
        "/api/approvals/appr_1/accept",
        Some(json!({"arguments": {"ok": true}})),
    )
    .await;
    request_json(
        app.clone(),
        "POST",
        "/api/inbox/iss_1/resolve",
        Some(json!({"note": "done"})),
    )
    .await;

    request_json(
        app.clone(),
        "DELETE",
        &format!("/api/agents/{agent_id}/memory/deploys"),
        None,
    )
    .await;
    request_json(
        app.clone(),
        "DELETE",
        &format!("/api/agents/{agent_id}/files/notes.txt"),
        None,
    )
    .await;
    request_json(app, "DELETE", &format!("/api/agents/{agent_id}"), None).await;
}

fn build_state(pool: PgPool) -> Arc<AppState> {
    let config = GatewayConfig {
        model_list: Vec::new(),
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            database_url: Some("postgres://test".to_owned()),
        },
    };
    let http = AppState::build_http_client().unwrap();
    Arc::new(
        AppState::new(
            config,
            ModelRouter::from_config(
                &GatewayConfig {
                    model_list: Vec::new(),
                    mcp_servers: HashMap::new(),
                    general_settings: GeneralSettings::default(),
                },
                &ProviderRegistry::new(),
            )
            .unwrap(),
            http,
            HashMap::new(),
            Some(pool),
        )
        .unwrap(),
    )
}

async fn request_json(app: axum::Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

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

async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
    content_type: &str,
    expected: StatusCode,
) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(body.unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), expected);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
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

async fn seed_inbox(pool: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_ManagedAgentInboxItemsTable"
          (id, kind, title, status, created_at)
        VALUES
          ('appr_1', 'approval', 'approve deploy', 'pending', 1),
          ('iss_1', 'issue', 'deployment issue', 'open', 2)
        "#,
    )
    .execute(pool)
    .await
    .unwrap();
}
