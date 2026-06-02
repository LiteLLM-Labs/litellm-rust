use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
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

pub fn app_for_e2b(e2b_api_base: String) -> axum::Router {
    router(build_state(&test_config(e2b_api_base)))
}

pub async fn mock_e2b() -> MockServer {
    let server = MockServer::start().await;
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
            "domain": server.uri()
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/process.Process/Start"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(connect_json_frames(process_frames())),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/sbx_test"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    server
}

pub async fn start_agent_run(app: &axum::Router) -> (String, String) {
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
    assert_eq!(event_url, "/event");
    let run_id = body["run_id"].as_str().unwrap();
    (event_url.to_owned(), run_id.to_owned())
}

pub async fn create_agent_session(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "title": "Untitled agent", "agent": "untitled-agent" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["agent_id"], "untitled-agent");
    body["id"].as_str().unwrap().to_owned()
}

pub async fn send_session_prompt(app: &axum::Router, session_id: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/prompt_async"))
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "parts": [{ "type": "text", "text": "please say hello" }] })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

pub async fn session_messages(app: &axum::Router, session_id: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/session/{session_id}/message"))
                .header(header::AUTHORIZATION, "Bearer sk-local")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

pub async fn read_events_until_completed(app: axum::Router, event_url: String) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{event_url}?key=sk-local"))
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
    collect_until_idle(response.into_body().into_data_stream()).await
}

async fn collect_until_idle(mut stream: axum::body::BodyDataStream) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut body = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            body.push_str(std::str::from_utf8(&chunk).unwrap());
            if body.contains("\"type\":\"session.idle\"") {
                break;
            }
        }
        body
    })
    .await
    .unwrap()
}

fn process_frames() -> Vec<Vec<u8>> {
    vec![
        br#"{"event":{"start":{"pid":1470}}}"#.to_vec(),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello "}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"from sandbox\n"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":""}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"thinking trace"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","name":"bash","input":{}}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"pwd\"}"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":2}}),
        ),
        stderr_frame(json!({"type":"text_delta","text":"npm notice\n"})),
        br#"{"event":{"end":{"exited":true,"status":"exit status 0"}}}"#.to_vec(),
    ]
}

fn stdout_frame(value: serde_json::Value) -> Vec<u8> {
    output_frame("stdout", value)
}

fn stderr_frame(value: serde_json::Value) -> Vec<u8> {
    output_frame("stderr", value)
}

fn output_frame(stream: &str, value: serde_json::Value) -> Vec<u8> {
    json!({ stream: BASE64_STANDARD.encode(format!("{value}\n")) })
        .to_string()
        .into_bytes()
}

fn connect_json_frames(payloads: Vec<Vec<u8>>) -> Vec<u8> {
    let mut frames = Vec::new();
    for payload in payloads.iter() {
        frames.push(0);
        frames.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frames.extend_from_slice(payload);
    }
    frames
}

fn test_config(e2b_api_base: String) -> GatewayConfig {
    GatewayConfig {
        model_list: Vec::new(),
        mcp_servers: HashMap::new(),
        general_settings: GeneralSettings {
            master_key: Some("sk-local".to_owned()),
            database_url: None,
            sandbox_choice: Some("e2b".to_owned()),
            e2b_sandbox_params: E2bSandboxParams {
                e2b_api_key: Some("e2b-test".to_owned()),
                e2b_template: "litellm-4gb".to_owned(),
                timeout_seconds: 1800,
                workspace_dir: "/home/user/workspace".to_owned(),
                e2b_api_base,
                envs: Default::default(),
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
    Arc::new(
        AppState::new(
            config.clone(),
            build_router(config),
            http,
            HashMap::new(),
            None,
        )
        .unwrap(),
    )
}
