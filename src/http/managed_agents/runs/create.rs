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
    http::{
        agent_runs::{parse_run_agent_request, start_agent_run},
        agents::has_configured_agent,
    },
    proxy::{auth::master_key::require_master_key, state::AppState},
};

use super::types::RunCreateResponse;

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(input): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    if has_configured_agent(&state, &agent_id) {
        return start_agent_run(state, agent_id, parse_run_agent_request(input)?, None).await;
    }

    let Some(pool) = state.db.as_ref() else {
        return Err(GatewayError::MissingDatabase);
    };
    let input: CreateRun = serde_json::from_value(input)?;
    let agent = registry::repository::get(pool, &agent_id)
        .await?
        .ok_or_else(|| GatewayError::NotFound("agent not found".to_owned()))?;
    let run = repository::create(pool, &agent_id, agent.session_id, input).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::to_value(RunCreateResponse {
            run_id: run.id,
            agent_id,
            session_id: run.session_id.unwrap_or_default(),
            status: run.status,
            event_url: "/event",
        })?),
    ))
}
