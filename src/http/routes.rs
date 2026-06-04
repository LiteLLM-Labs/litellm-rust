use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        agents::events,
        health::health,
        messages::messages,
        models::models,
        openapi::{openapi_json, swagger_ui},
        responses::responses,
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
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .merge(crate::http::managed_agents::routes::router())
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
