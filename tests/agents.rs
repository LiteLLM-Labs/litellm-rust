use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use futures_util::StreamExt;
use litellm_rust::{
    agents::config::{AgentDefinition, E2bSandboxParams},
    http::routes::router,
    proxy::{
        config::{GatewayConfig, GeneralSettings},
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

#[tokio::test]
async fn starts_agent_and_streams_e2b_output() {
    let e2b = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "templateID": "litellm-4gb",
            "sandboxID": "sbx_test",
            "clientID": "client_test",
            "envdVersion": "test",
            "alias": "base",
            "envdAccessToken": "envd-test",
            "trafficAccessToken": "traffic-test",
            "domain": e2b.uri()
        })))
        .mount(&e2b)
        .await;
    Mock::given(method("POST"))
        .and(path("/process.Process/Start"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"stdout":"hello from sandbox\n"}"#),
        )
        .mount(&e2b)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/sbx_test"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&e2b)
        .await;

    let config = test_config(e2b.uri());
    let app = router(build_state(&config));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/untitled-agent/run")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "prompt": "say hello" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let event_url = body["event_url"].as_str().unwrap();
    let run_id = body["run_id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(event_url)
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let mut stream = response.into_body().into_data_stream();
    let body = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut body = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            body.push_str(std::str::from_utf8(&chunk).unwrap());
            if body.contains("agent.run.completed") {
                break;
            }
        }
        body
    })
    .await
    .unwrap();

    assert!(body.contains("agent.run.started"));
    assert!(body.contains("agent.execution.started"));
    assert!(body.contains("agent.output.delta"));
    assert!(body.contains("hello from sandbox"));
    assert!(body.contains("\"stream\":\"stdout\""));
    assert!(body.contains("agent.run.completed"));
    assert!(body.contains(run_id));
}

fn test_config(e2b_api_base: String) -> GatewayConfig {
    GatewayConfig {
        model_list: Vec::new(),
        mcp_servers: Vec::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            sandbox_choice: Some("e2b".to_owned()),
            e2b_sandbox_params: E2bSandboxParams {
                e2b_api_key: Some("e2b-test".to_owned()),
                e2b_template: "litellm-4gb".to_owned(),
                timeout_seconds: 1800,
                workspace_dir: "/home/user/workspace".to_owned(),
                e2b_api_base,
            },
        },
        agents: vec![AgentDefinition {
            id: None,
            name: "Untitled agent".to_owned(),
            description: Some("A blank starting point with the core toolset.".to_owned()),
            model: "claude-sonnet-4-6".to_owned(),
            harness: None,
            system: "You are a general-purpose agent.".to_owned(),
            mcp_servers: Vec::new(),
            tools: vec![HashMap::from([(
                "type".to_owned(),
                serde_yaml::Value::String("agent_toolset_20260401".to_owned()),
            )])],
            skills: Vec::new(),
        }],
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
