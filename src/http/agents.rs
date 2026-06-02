use std::{collections::VecDeque, convert::Infallible, sync::Arc};

use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::{
    agents::{
        config::AgentDefinition,
        events,
        harnesses::build_harness_run,
        runs::AgentRunStatus,
        sandboxes::{SandboxCommand, SandboxRunner},
    },
    errors::GatewayError,
    proxy::{auth::master_key::require_master_key, state::AppState},
};

pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;

    Ok(Json(json!({
        "agents": state.config.agents.iter().map(AgentResponse::from).collect::<Vec<_>>()
    })))
}

pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;

    Ok(Json(json!(AgentResponse::from(find_agent(
        &state, &agent_id
    )?))))
}

pub async fn events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, GatewayError> {
    require_master_key(
        &headers,
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

pub async fn run_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(body): Json<RunAgentRequest>,
) -> Result<impl IntoResponse, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;

    let agent = find_agent(&state, &agent_id)?.clone();

    let prompt = body
        .prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or_else(|| "Proceed with your task.".to_owned());
    let run = state.agent_runs.create_run(&agent_id);
    let run_id = run.id.clone();

    spawn_agent_run(state.clone(), agent, prompt, run_id.clone());

    Ok((
        StatusCode::ACCEPTED,
        Json(RunAgentResponse {
            run_id,
            agent_id,
            status: "starting",
            event_url: "/events",
        }),
    ))
}

pub async fn list_agent_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    find_agent(&state, &agent_id)?;

    Ok(Json(
        json!({ "runs": state.agent_runs.list_runs(&agent_id) }),
    ))
}

fn spawn_agent_run(state: Arc<AppState>, agent: AgentDefinition, prompt: String, run_id: String) {
    tokio::spawn(async move {
        if let Err(error) = execute_agent_run(state.clone(), agent, prompt, &run_id).await {
            let message = error.to_string();
            state.agent_runs.set_error(&run_id, message.clone());
            state
                .agent_runs
                .push_event(&run_id, events::RUN_FAILED, json!({ "error": message }));
        }
    });
}

async fn execute_agent_run(
    state: Arc<AppState>,
    agent: AgentDefinition,
    prompt: String,
    run_id: &str,
) -> Result<(), GatewayError> {
    let store = &state.agent_runs;
    store.push_event(run_id, events::RUN_STARTED, json!({ "run_id": run_id }));

    let harness_run = build_harness_run(&agent, &prompt)?;
    let sandbox = SandboxRunner::from_settings(state.http.clone(), &state.config.general_settings)?;
    let session = sandbox.create(run_id).await?;
    if let Some(sandbox_id) = session.sandbox_id.clone() {
        store.set_sandbox_id(run_id, sandbox_id);
    }
    store.update_status(run_id, AgentRunStatus::Running);
    store.push_event(
        run_id,
        events::EXECUTION_STARTED,
        json!({
            "target_id": session.sandbox_id,
            "target_kind": session.target_kind.as_str(),
            "target_provider": session.provider,
        }),
    );
    let run_result = async {
        let mut stream = sandbox
            .start(
                &session,
                SandboxCommand {
                    command: harness_run.command,
                },
            )
            .await?;
        while let Some(output) = stream.next().await {
            let output = output?;
            if output.delta.is_empty() {
                continue;
            }
            store.push_event(
                run_id,
                events::OUTPUT_DELTA,
                json!({ "delta": output.delta, "stream": output.stream.as_str() }),
            );
        }
        Ok::<(), GatewayError>(())
    }
    .await;

    let _ = sandbox.terminate(&session).await;
    run_result?;

    store.update_status(run_id, AgentRunStatus::Completed);
    store.push_event(run_id, events::RUN_COMPLETED, json!({ "run_id": run_id }));
    Ok(())
}

fn find_agent<'a>(
    state: &'a AppState,
    agent_id: &str,
) -> Result<&'a AgentDefinition, GatewayError> {
    state
        .config
        .agents
        .iter()
        .find(|agent| agent.id() == agent_id)
        .ok_or_else(|| GatewayError::UnknownAgent(agent_id.to_owned()))
}

#[derive(Debug, Deserialize)]
pub struct RunAgentRequest {
    pub prompt: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunAgentResponse<'a> {
    run_id: String,
    agent_id: String,
    status: &'a str,
    event_url: &'a str,
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
