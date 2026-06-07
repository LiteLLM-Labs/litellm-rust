use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::{
    http::{
        agents::events,
        capabilities::capabilities,
        chat_completions::chat_completions,
        gemini,
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
        .route("/api/capabilities", get(capabilities))
        .route(
            "/api/providers",
            get(crate::http::provider_credentials::list),
        )
        .route(
            "/api/providers/{provider_id}",
            post(crate::http::provider_credentials::save_provider)
                .delete(crate::http::provider_credentials::delete_provider),
        )
        .route("/v1/messages", post(messages))
        .route("/v1/responses", post(responses))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1beta/models/{model_method}", post(gemini::generate))
        .route("/v1/models", get(models))
        .merge(crate::http::management::routes::router())
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
