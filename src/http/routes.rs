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
    proxy::state::AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(redirect_to_docs))
        .route("/docs", get(swagger_ui))
        .route("/openapi.json", get(openapi_json))
        .route("/health", get(health))
        .route("/v1/messages", post(messages))
        .with_state(state)
}
