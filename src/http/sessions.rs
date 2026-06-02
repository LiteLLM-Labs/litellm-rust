use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    errors::GatewayError,
    http::{
        agent_runs::{resolve_agent_definition, start_agent_run, RunAgentRequest},
        agents::default_config_agent_id,
    },
    proxy::{auth::master_key::require_master_key, state::AppState},
};

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    Ok(Json(serde_json::to_value(state.agent_sessions.list())?))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    let agent_id = selected_agent_id(&state, input.agent).await?;
    let agent = resolve_agent_definition(&state, &agent_id).await?;
    let title = input.title.filter(|title| !title.trim().is_empty());
    let harness = agent.resolved_harness().to_owned();
    let session = state.agent_sessions.create(
        title.or_else(|| Some(agent.name.clone())),
        agent_id,
        harness,
    );
    Ok((StatusCode::CREATED, Json(serde_json::to_value(session)?)))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    let session = state
        .agent_sessions
        .get(&session_id)
        .ok_or_else(|| GatewayError::NotFound("session not found".to_owned()))?;
    Ok(Json(serde_json::to_value(session)?))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    state.agent_sessions.delete(&session_id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    let messages = state
        .agent_sessions
        .messages(&session_id)
        .ok_or_else(|| GatewayError::NotFound("session not found".to_owned()))?;
    Ok(Json(serde_json::to_value(messages)?))
}

pub async fn prompt_async(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(input): Json<Value>,
) -> Result<Response, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    let session = state
        .agent_sessions
        .get(&session_id)
        .ok_or_else(|| GatewayError::NotFound("session not found".to_owned()))?;
    let prompt = prompt_text(&input);
    if prompt.trim().is_empty() {
        return Err(GatewayError::InvalidJsonMessage(
            "prompt text is required".to_owned(),
        ));
    }
    state.agent_sessions.push_user_message(&session_id, &prompt);
    let _started = start_agent_run(
        state,
        session.agent_id,
        RunAgentRequest {
            prompt: Some(prompt),
        },
        Some(session_id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn abort(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    if state.agent_sessions.get(&session_id).is_none() {
        return Err(GatewayError::NotFound("session not found".to_owned()));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub agent: Option<String>,
}

async fn selected_agent_id(
    state: &AppState,
    requested: Option<String>,
) -> Result<String, GatewayError> {
    if let Some(agent_id) = requested.filter(|id| !id.trim().is_empty()) {
        match resolve_agent_definition(state, &agent_id).await {
            Ok(_) => return Ok(agent_id),
            Err(GatewayError::UnknownAgent(_)) => {}
            Err(error) => return Err(error),
        }
    }
    default_config_agent_id(state)
        .ok_or_else(|| GatewayError::UnknownAgent("no configured agent".to_owned()))
}

fn prompt_text(input: &Value) -> String {
    input
        .get("parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            input
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}
