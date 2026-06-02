use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use serde_json::json;

use crate::proxy::{auth::master_key::require_master_key, state::AppState};

pub async fn whoami(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    require_master_key(&headers, state.config.general_settings.master_key.as_deref())?;
    Ok::<_, crate::errors::GatewayError>(Json(json!({ "status": "ok" })))
}
