use std::{collections::HashMap, sync::Arc};

use axum::{http::StatusCode, Json};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::{
    agents::{
        config::AgentDefinition,
        events,
        harnesses::{build_harness_run, HarnessEvent, HarnessRunContext},
        runs::{AgentRunStatus, AgentRunStore},
        sandboxes::{SandboxCommand, SandboxRunner},
    },
    db::managed_agents::registry::{repository as agent_repository, schema::ManagedAgentRow},
    errors::GatewayError,
    proxy::state::AppState,
};

pub async fn resolve_agent_definition(
    state: &AppState,
    agent_id: &str,
) -> Result<AgentDefinition, GatewayError> {
    if let Some(agent) = state
        .config
        .agents
        .iter()
        .find(|agent| agent.id() == agent_id)
    {
        return Ok(agent.clone());
    }

    let Some(pool) = state.db.as_ref() else {
        return Err(GatewayError::UnknownAgent(agent_id.to_owned()));
    };
    let row = agent_repository::get(pool, agent_id)
        .await?
        .ok_or_else(|| GatewayError::UnknownAgent(agent_id.to_owned()))?;
    Ok(managed_agent_definition(row))
}

pub async fn start_agent_run(
    state: Arc<AppState>,
    agent_id: String,
    body: RunAgentRequest,
    session_id: Option<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), GatewayError> {
    let agent = resolve_agent_definition(&state, &agent_id).await?;
    let prompt = body
        .prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or_else(|| "Proceed with your task.".to_owned());
    let run = state.agent_runs.create_run(&agent_id);
    let run_id = run.id.clone();
    let event_session_id = session_id.unwrap_or_else(|| run_id.clone());

    spawn_agent_run(
        state.clone(),
        agent,
        prompt,
        run_id.clone(),
        event_session_id,
    );

    let response = RunAgentResponse {
        run_id,
        agent_id,
        status: "starting",
        event_url: "/event",
    };
    Ok((StatusCode::ACCEPTED, Json(serde_json::to_value(response)?)))
}

pub fn parse_run_agent_request(value: serde_json::Value) -> Result<RunAgentRequest, GatewayError> {
    serde_json::from_value(value).map_err(GatewayError::InvalidJson)
}

fn spawn_agent_run(
    state: Arc<AppState>,
    agent: AgentDefinition,
    prompt: String,
    run_id: String,
    session_id: String,
) {
    tokio::spawn(async move {
        if let Err(error) =
            execute_agent_run(state.clone(), agent, prompt, &run_id, &session_id).await
        {
            publish_error(&state, &run_id, &session_id, error.to_string());
        }
    });
}

fn publish_error(state: &AppState, run_id: &str, session_id: &str, message: String) {
    state.agent_runs.set_error(run_id, message.clone());
    let error_event = json!({ "error": { "message": message }, "sessionID": session_id });
    state
        .agent_sessions
        .apply_event(session_id, events::SESSION_ERROR, &error_event);
    state
        .agent_runs
        .push_event(run_id, events::SESSION_ERROR, error_event);
    state.agent_runs.push_event(
        run_id,
        events::SESSION_IDLE,
        json!({ "sessionID": session_id }),
    );
}

async fn execute_agent_run(
    state: Arc<AppState>,
    agent: AgentDefinition,
    prompt: String,
    run_id: &str,
    session_id: &str,
) -> Result<(), GatewayError> {
    let store = &state.agent_runs;
    let mut harness_run = build_harness_run(&agent, &prompt)?;
    let context = HarnessRunContext::for_session(run_id, session_id);
    push_harness_events(
        &state,
        store,
        run_id,
        session_id,
        harness_run.events.start(&context),
    );

    let sandbox = SandboxRunner::from_settings(state.http.clone(), &state.config.general_settings)?;
    let session = sandbox.create(run_id).await?;
    if let Some(sandbox_id) = session.sandbox_id.clone() {
        store.set_sandbox_id(run_id, sandbox_id);
    }
    store.update_status(run_id, AgentRunStatus::Running);
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
            let events = harness_run.events.output(&context, output);
            push_harness_events(&state, store, run_id, session_id, events);
        }
        Ok::<(), GatewayError>(())
    }
    .await;

    let _ = sandbox.terminate(&session).await;
    run_result?;

    store.update_status(run_id, AgentRunStatus::Completed);
    push_harness_events(
        &state,
        store,
        run_id,
        session_id,
        harness_run.events.complete(&context),
    );
    Ok(())
}

fn push_harness_events(
    state: &AppState,
    store: &AgentRunStore,
    run_id: &str,
    session_id: &str,
    events: Vec<HarnessEvent>,
) {
    for event in events {
        state
            .agent_sessions
            .apply_event(session_id, event.event, &event.data);
        store.push_event(run_id, event.event, event.data);
    }
}

fn managed_agent_definition(agent: ManagedAgentRow) -> AgentDefinition {
    AgentDefinition {
        id: Some(agent.id),
        name: agent.name,
        description: agent.description,
        model: agent.model,
        harness: Some(agent.harness),
        system: agent.system,
        mcp_servers: Vec::new(),
        tools: vec![HashMap::from([(
            "type".to_owned(),
            serde_yaml::Value::String("agent_toolset_20260401".to_owned()),
        )])],
        skills: Vec::new(),
    }
}

#[derive(Debug, Deserialize)]
pub struct RunAgentRequest {
    pub prompt: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct RunAgentResponse<'a> {
    run_id: String,
    agent_id: String,
    status: &'a str,
    event_url: &'a str,
}
