use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    proxy::state::AppState,
    http::{health::health, messages::messages},
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/messages", post(messages))
        .with_state(state)
}
