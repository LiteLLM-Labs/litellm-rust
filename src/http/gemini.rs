use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue},
    response::Response,
};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    http::pipeline,
    proxy::{auth::master_key::require_any_gateway_key, state::AppState},
    sdk::codec::WireFormat,
};

/// Native Gemini endpoint: `POST /v1beta/models/{model}:generateContent` and
/// `:streamGenerateContent`. The model and streaming variant come from the path;
/// auth may arrive via `x-goog-api-key`, `?key=`, or a standard bearer token.
pub async fn generate(
    State(state): State<Arc<AppState>>,
    Path(model_method): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    mut headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    let scope_key = authorize(&state, &headers, params.get("key").map(String::as_str))?;

    // Pin the credential that actually authenticated as the bearer, so cache scoping
    // (`presented_key`: Authorization > x-api-key > x-goog-api-key) reflects it
    // rather than a stale/dummy higher-priority header. Gemini's outbound auth
    // ignores inbound headers, so this only affects scoping.
    if let Some(key) = scope_key {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {key}")) {
            headers.insert(axum::http::header::AUTHORIZATION, value);
        }
    }

    let (model, method) = model_method.split_once(':').ok_or_else(|| {
        GatewayError::InvalidJsonMessage("gemini path must be models/{model}:{method}".to_owned())
    })?;
    let stream = match method {
        "generateContent" => false,
        "streamGenerateContent" => true,
        other => {
            return Err(GatewayError::InvalidJsonMessage(format!(
                "unsupported gemini method: {other}"
            )))
        }
    };

    let body: Value = serde_json::from_slice(&body).map_err(GatewayError::InvalidJson)?;

    pipeline::handle(
        &state,
        WireFormat::Gemini,
        model.to_owned(),
        stream,
        body,
        &headers,
    )
    .await
}

/// Returns the credential that authenticated the request, if it needs to be pinned
/// for cache scoping (the accepted `x-goog-api-key` or `?key=`, which `presented_key`
/// would otherwise mis-prioritise). Returns `None` when auth is off or a standard
/// gateway header authenticated (which `presented_key` already reads correctly).
fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    query_key: Option<&str>,
) -> Result<Option<String>, GatewayError> {
    let Some(master) = state.config.general_settings.master_key.as_deref() else {
        return Ok(None);
    };
    let accepted = |k: &str| k == master || state.api_keys.accepts(k);
    if let Some(k) = headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()) {
        if accepted(k) {
            return Ok(Some(k.to_owned()));
        }
    }
    if let Some(k) = query_key {
        if accepted(k) {
            return Ok(Some(k.to_owned()));
        }
    }
    require_any_gateway_key(headers, state)?;
    Ok(None)
}
