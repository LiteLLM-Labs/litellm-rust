use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        health::health,
        messages::messages,
        openapi::{openapi_json, swagger_ui},
        ui,
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
        .route("/whoami", get(ui::whoami))
        .route("/_litellm/health", get(ui::litellm_health))
        .route("/v1/models", get(ui::models))
        .route("/v1/messages", post(messages))
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
        .route("/session", get(ui::sessions).post(ui::create_session))
        .route("/session/{id}", get(ui::session).delete(ui::delete_session))
        .route("/session/{id}/message", get(ui::session_messages))
        .route("/session/{id}/prompt_async", post(ui::prompt_async))
        .route("/session/{id}/abort", post(ui::abort_session))
        .route("/event", get(ui::events))
        .route("/api/agents", get(ui::agents))
        .route("/api/approvals", get(ui::approvals))
        .route("/api/inbox", get(ui::inbox))
        .route("/api/skills", get(ui::skills))
        .route("/api/vault/{user_id}", get(ui::vault))
        .fallback_service(ui::static_files())
        .with_state(state)
}
