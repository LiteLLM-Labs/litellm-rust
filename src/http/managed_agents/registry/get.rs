use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use crate::{
    db::managed_agents::registry::{repository, schema::ManagedAgentRow},
    errors::GatewayError,
    proxy::state::AppState,
};

pub async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<ManagedAgentRow>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    let row = repository::get(pool, &agent_id)
        .await?
        .ok_or_else(|| GatewayError::NotFound("not found".to_owned()))?;
    Ok(Json(row))
}
