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
        .with_state(state)
}
