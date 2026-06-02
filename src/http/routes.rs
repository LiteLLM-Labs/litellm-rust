use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        health::health,
        mcp_credentials,
        messages::messages,
        openapi::{openapi_json, swagger_ui},
        ui, users,
    },
    mcp::route::{streamable_http, streamable_http_server},
    proxy::state::AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(ui::redirect_to_sessions))
        .route("/docs", get(swagger_ui))
        .route("/openapi.json", get(openapi_json))
        .route("/health", get(health))
        .route("/v1/messages", post(messages))
        .merge(crate::http::managed_agents::routes::router())
        .merge(user_and_credential_routes())
        .route(
            "/mcp",
            get(streamable_http)
                .post(streamable_http)
                .delete(streamable_http),
        )
        .route(
            "/mcp/{server_id}",
            get(streamable_http_server)
                .post(streamable_http_server)
                .delete(streamable_http_server),
        )
        .fallback_service(ui::static_files())
        .with_state(state)
}

/// Per-user identity + BYOK MCP credential endpoints (LiteLLM-compatible).
fn user_and_credential_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/user/new", post(users::new_user))
        .route("/key/generate", post(users::generate_key))
        .route("/v1/mcp/user-credentials", get(mcp_credentials::list))
        .route(
            "/v1/mcp/server/{server_id}/user-credential",
            post(mcp_credentials::put_static)
                .get(mcp_credentials::status)
                .delete(mcp_credentials::delete),
        )
        .route(
            "/v1/mcp/server/{server_id}/oauth-user-credential",
            post(mcp_credentials::put_oauth).delete(mcp_credentials::delete),
        )
        .route(
            "/v1/mcp/server/{server_id}/oauth-user-credential/status",
            get(mcp_credentials::status),
        )
}
