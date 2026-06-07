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
    authorize(&state, &headers, params.get("key").map(String::as_str))?;

    // `?key=` is an accepted Gemini credential, but cache scoping reads only
    // headers. Pin `x-goog-api-key` to the query key (overwriting any stale/dummy
    // header) so credential scoping reflects the key that actually authenticated —
    // otherwise distinct query-key callers sharing a dummy header could share cache.
    if let Some(key) = params.get("key") {
        if let Ok(value) = HeaderValue::from_str(key) {
            headers.insert("x-goog-api-key", value);
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

fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    query_key: Option<&str>,
) -> Result<(), GatewayError> {
    let Some(master) = state.config.general_settings.master_key.as_deref() else {
        return Ok(());
    };
    let accepted = |k: &str| k == master || state.api_keys.accepts(k);
    if let Some(k) = headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()) {
        if accepted(k) {
            return Ok(());
        }
    }
    if let Some(k) = query_key {
        if accepted(k) {
            return Ok(());
        }
    }
    require_any_gateway_key(headers, state)
}
