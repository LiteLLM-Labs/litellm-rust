use std::sync::Arc;

use axum::{body::Bytes, extract::State, http::HeaderMap, response::Response};
use serde_json::Value;
use tracing::{error, info};

use crate::{
    errors::GatewayError,
    http::llm,
    proxy::{auth::master_key::require_master_key, state::AppState},
};

pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )
    .map_err(|e| {
        error!(error = %e, "request rejected: unauthorized");
        e
    })?;

    let body: Value = serde_json::from_slice(&body).map_err(GatewayError::InvalidJson)?;
    let model: String = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or(GatewayError::MissingModel)?
        .to_owned();

    info!(model, "request received");

    let route = state.router.resolve(&model).map_err(|e| {
        error!(model, error = %e, "model routing failed");
        e
    })?;

    let prepared = route
        .handler
        .transform_request(body, &route.deployment, &headers)
        .map_err(|e| {
            error!(model, error = %e, "request transform failed");
            e
        })?;
    let stream = prepared.stream;

    let upstream = llm::send_request(&state.http, route.deployment.messages_url(), prepared)
        .await
        .map_err(|e| {
            error!(model, error = %e, "upstream request failed");
            e
        })?;

    info!(model, stream, "upstream response received");

    let response_headers = route
        .handler
        .transform_response_headers(upstream.headers(), stream);
    Ok(llm::build_response(upstream, response_headers).await)
}
