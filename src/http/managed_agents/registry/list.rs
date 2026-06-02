use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};

use crate::{
    db::managed_agents::registry::{repository, schema::ManagedAgentRow},
    errors::GatewayError,
    proxy::state::AppState,
};

use super::types::{AgentsResponse, ListAgentsQuery};

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<AgentsResponse<Vec<ManagedAgentRow>>>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    let agents = repository::list(pool, query.owner_id.as_deref()).await?;
    Ok(Json(AgentsResponse { agents }))
}
