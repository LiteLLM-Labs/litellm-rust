//! Integration tests for per-user (BYOK) MCP credentials.
//!
//! Proves the full journey: create a user, store their upstream credential, and
//! have the gateway inject it into an MCP call — asserted against a mock
//! upstream. Requires `TEST_DATABASE_URL`; skipped otherwise.

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use litellm_rust::{
    db::managed_agents::pool as managed_agents_pool,
    http::routes::router,
    proxy::{
        config::{GatewayConfig, GeneralSettings, McpAuthType, McpServerEntry, McpTransport},
        state::AppState,
    },
    sdk::{providers::transform::ProviderRegistry, router::Router as ModelRouter},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::util::ServiceExt;
use wiremock::{
    matchers::{header as header_match, method, path},
    Mock, MockServer, ResponseTemplate,
};

const ADMIN_KEY: &str = "sk-local";
const USER_TOKEN: &str = "ya29.test-access-token";

struct Fixture {
    app: axum::Router,
}

impl Fixture {
    /// Build an app whose only MCP server is a BYOK `gmail` pointing at `mcp_url`.
    async fn new(pool: PgPool, mcp_url: String) -> Self {
        reset_tables(&pool).await;
        let mut mcp_servers = HashMap::new();
        mcp_servers.insert(
            "gmail".to_owned(),
            McpServerEntry {
                url: mcp_url,
                transport: McpTransport::Http,
                auth_type: McpAuthType::BearerToken,
                auth_value: None,
                static_headers: HashMap::new(),
                extra_headers: Vec::new(),
                description: Some("Gmail".to_owned()),
                is_byok: true,
                byok_description: vec!["Gmail OAuth token".to_owned()],
                byok_api_key_help_url: None,
            },
        );
        let config = GatewayConfig {
            model_list: Vec::new(),
            mcp_servers,
            general_settings: GeneralSettings {
                master_key: Some(ADMIN_KEY.to_owned()),
                database_url: Some("postgres://test".to_owned()),
            },
        };
        let http = AppState::build_http_client().unwrap();
        let state = Arc::new(
            AppState::new(config, empty_router(), http, HashMap::new(), Some(pool)).unwrap(),
        );
        Self { app: router(state) }
    }

    async fn send(
        &self,
        method: &str,
        uri: &str,
        bearer: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }
}

#[tokio::test]
async fn byok_credential_is_injected_into_mcp_call() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL not set");
        return;
    };

    // Mock Gmail MCP server: only responds when the gateway forwarded the user's
    // bearer token. A request without it does not match and yields 404 upstream.
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header_match(
            "authorization",
            format!("Bearer {USER_TOKEN}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [ { "name": "gmail_search" } ] }
        })))
        .mount(&upstream)
        .await;

    let fixture = Fixture::new(pool, format!("{}/mcp", upstream.uri())).await;
    let tools_list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});

    // 1. Admin creates a user (returns a user API key).
    let (status, body) = fixture
        .send(
            "POST",
            "/user/new",
            ADMIN_KEY,
            Some(json!({"user_alias": "alice"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "user/new: {body}");
    let user_key = body["key"].as_str().expect("key in response").to_owned();

    // 2. Before storing a credential, the BYOK MCP call is rejected.
    let (status, _) = fixture
        .send("POST", "/mcp/gmail", &user_key, Some(tools_list.clone()))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "missing credential must 401"
    );

    // 3. User stores their Gmail token.
    let (status, _) = fixture
        .send(
            "POST",
            "/v1/mcp/server/gmail/user-credential",
            &user_key,
            Some(json!({"credential": USER_TOKEN})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "store credential");

    // 4. Status now reports a stored credential.
    let (status, body) = fixture
        .send(
            "GET",
            "/v1/mcp/server/gmail/user-credential",
            &user_key,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["has_credential"], json!(true), "{body}");

    // 5. The MCP call now succeeds — the gateway injected the user's token, which
    //    is the only way the mock upstream returns 200.
    let (status, body) = fixture
        .send("POST", "/mcp/gmail", &user_key, Some(tools_list.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "injected call: {body}");
    assert_eq!(body["result"]["tools"][0]["name"], json!("gmail_search"));

    // 6. The admin master key is not a user and cannot use a BYOK server.
    let (status, _) = fixture
        .send("POST", "/mcp/gmail", ADMIN_KEY, Some(tools_list))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "admin key on BYOK must 401"
    );
}

async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = managed_agents_pool::connect(&url).await.unwrap();
    managed_agents_pool::migrate(&pool).await.unwrap();
    Some(pool)
}

fn empty_router() -> ModelRouter {
    ModelRouter::from_config(
        &GatewayConfig {
            model_list: Vec::new(),
            mcp_servers: HashMap::new(),
            general_settings: GeneralSettings::default(),
        },
        &ProviderRegistry::new(),
    )
    .unwrap()
}

async fn reset_tables(pool: &PgPool) {
    sqlx::query(
        r#"TRUNCATE
             "LiteLLM_MCPUserCredentialTable",
             "LiteLLM_VerificationTokenTable",
             "LiteLLM_UserTable"
           CASCADE"#,
    )
    .execute(pool)
    .await
    .unwrap();
}
