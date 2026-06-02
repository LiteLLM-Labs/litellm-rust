use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};

use crate::{db::managed_agents::runs::repository, errors::GatewayError, proxy::state::AppState};

use super::types::{ListRunsQuery, RunsResponse};

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<RunsResponse>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    Ok(Json(RunsResponse {
        runs: repository::list(pool, &agent_id, query.limit.unwrap_or(10)).await?,
    }))
}
