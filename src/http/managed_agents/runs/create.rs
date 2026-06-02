use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    db::managed_agents::{
        registry,
        runs::{repository, schema::CreateRun},
    },
    errors::GatewayError,
    proxy::state::AppState,
};

use super::types::RunCreateResponse;

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(input): Json<CreateRun>,
) -> Result<(StatusCode, Json<RunCreateResponse>), GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    let agent = registry::repository::get(pool, &agent_id)
        .await?
        .ok_or_else(|| GatewayError::NotFound("agent not found".to_owned()))?;
    let run = repository::create(pool, &agent_id, agent.session_id, input).await?;
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let logs_url = format!("http://{host}/api/agents/{agent_id}/runs/{}/logs", run.id);
    Ok((
        StatusCode::ACCEPTED,
        Json(RunCreateResponse {
            run_id: run.id,
            agent_id,
            session_id: run.session_id.unwrap_or_default(),
            status: run.status,
            logs_url,
        }),
    ))
}
