use std::sync::Arc;

use axum::{body::Bytes, extract::State, http::HeaderMap, response::Response};

use crate::{
    ai_gateway::messages::{MessagesCreateRequest, MessagesGateway},
    app::{errors::GatewayError, state::AppState},
    auth::master_key::require_master_key,
};

pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;

    MessagesGateway::new(&state.http, &state.models)
        .create(MessagesCreateRequest { headers, body })
        .await
}
