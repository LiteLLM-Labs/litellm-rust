use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        agents::{events, get_agent, list_agent_runs, list_agents, run_agent},
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
        .route("/event", get(events))
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
        .route("/api/agents", get(list_agents))
        .route("/api/agents/{agent_id}", get(get_agent))
        .route("/api/agents/{agent_id}/run", post(run_agent))
        .route("/api/agents/{agent_id}/runs", get(list_agent_runs))
        .fallback_service(ui::static_files())
        .with_state(state)
}
