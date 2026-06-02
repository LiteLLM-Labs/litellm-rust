use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        health::health,
        messages::messages,
        openapi::{openapi_json, redirect_to_docs, swagger_ui},
        ui,
        whoami::whoami,
    },
    mcp::route::{streamable_http, streamable_http_server},
    proxy::state::AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(redirect_to_docs))
        .route("/docs", get(swagger_ui))
        .route("/openapi.json", get(openapi_json))
        .route("/health", get(health))
        .route("/whoami", get(whoami))
        .route("/v1/messages", post(messages))
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
        .nest_service("/ui", ui::static_files())
        .with_state(state)
}
