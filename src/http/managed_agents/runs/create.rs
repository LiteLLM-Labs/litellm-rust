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
    http::agents::{has_configured_agent, parse_run_agent_request, start_configured_agent_run},
    proxy::{auth::master_key::require_any_gateway_key, state::AppState},
};

use super::types::RunCreateResponse;

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(input): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), GatewayError> {
    require_any_gateway_key(&headers, &state)?;
    if has_configured_agent(&state, &agent_id) {
        return start_configured_agent_run(state, agent_id, parse_run_agent_request(input)?);
    }

    let Some(pool) = state.db.as_ref() else {
        return Err(GatewayError::MissingDatabase);
    };
    let input: CreateRun = serde_json::from_value(input)?;
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
        Json(serde_json::to_value(RunCreateResponse {
            run_id: run.id,
            agent_id,
            session_id: run.session_id.unwrap_or_default(),
            status: run.status,
            logs_url,
        })?),
    ))
}
