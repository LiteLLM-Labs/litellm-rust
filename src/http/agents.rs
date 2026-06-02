use std::{collections::VecDeque, convert::Infallible, sync::Arc};

use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::HeaderMap,
    response::Response,
};
use futures_util::stream;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;

use crate::{
    agents::config::AgentDefinition,
    errors::GatewayError,
    proxy::{auth::master_key::require_master_key, state::AppState},
};

pub async fn events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, GatewayError> {
    require_events_master_key(
        &headers,
        &query,
        state.config.general_settings.master_key.as_deref(),
    )?;

    let events = state.agent_runs.event_stream();
    let body_stream = stream::unfold(
        (VecDeque::from(events.snapshot), events.rx),
        |(mut snapshot, mut rx)| async move {
            if let Some(line) = snapshot.pop_front() {
                return Some((Ok::<Bytes, Infallible>(Bytes::from(line)), (snapshot, rx)));
            }
            loop {
                match rx.recv().await {
                    Ok(line) => return Some((Ok(Bytes::from(line)), (snapshot, rx))),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(body_stream))
        .map_err(|error| GatewayError::SandboxError(error.to_string()))
}

fn require_events_master_key(
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    configured: Option<&str>,
) -> Result<(), GatewayError> {
    if query.get("key").map(String::as_str) == configured {
        return Ok(());
    }
    require_master_key(headers, configured)
}

pub fn configured_agent_values(state: &AppState) -> Vec<serde_json::Value> {
    state
        .config
        .agents
        .iter()
        .map(|agent| json!(AgentResponse::from(agent)))
        .collect()
}

pub fn configured_agent_value(state: &AppState, agent_id: &str) -> Option<serde_json::Value> {
    state
        .config
        .agents
        .iter()
        .find(|agent| agent.id() == agent_id)
        .map(|agent| json!(AgentResponse::from(agent)))
}

pub fn has_configured_agent(state: &AppState, agent_id: &str) -> bool {
    state
        .config
        .agents
        .iter()
        .any(|agent| agent.id() == agent_id)
}

pub fn configured_agent_runs_value(state: &AppState, agent_id: &str) -> Option<serde_json::Value> {
    has_configured_agent(state, agent_id)
        .then(|| json!({ "runs": state.agent_runs.list_runs(agent_id) }))
}

pub fn default_config_agent_id(state: &AppState) -> Option<String> {
    state.config.agents.first().map(AgentDefinition::id)
}

#[derive(Debug, Serialize)]
struct AgentResponse<'a> {
    id: String,
    name: &'a str,
    description: Option<&'a str>,
    model: &'a str,
    harness: &'a str,
    system: &'a str,
    mcp_servers: &'a [serde_yaml::Value],
    tools: &'a [HashMap<String, serde_yaml::Value>],
    skills: &'a [serde_yaml::Value],
}

impl<'a> From<&'a AgentDefinition> for AgentResponse<'a> {
    fn from(agent: &'a AgentDefinition) -> Self {
        Self {
            id: agent.id(),
            name: &agent.name,
            description: agent.description.as_deref(),
            model: &agent.model,
            harness: agent.resolved_harness(),
            system: &agent.system,
            mcp_servers: &agent.mcp_servers,
            tools: &agent.tools,
            skills: &agent.skills,
        }
    }
}
