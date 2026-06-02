pub mod claude_code;

use serde_json::Value;

use crate::{
    agents::{
        config::AgentDefinition,
        sandboxes::{AgentOutputChunk, AgentOutputStreamKind},
    },
    errors::GatewayError,
};

#[derive(Debug, Clone)]
pub struct HarnessRunSpec {
    pub command: String,
    pub events: HarnessEvents,
}

#[derive(Debug, Clone)]
pub enum HarnessEvents {
    ClaudeCode(claude_code::ClaudeCodeEvents),
}

impl HarnessEvents {
    pub fn start(&self, context: &HarnessRunContext) -> Vec<HarnessEvent> {
        match self {
            Self::ClaudeCode(events) => events.start(context),
        }
    }

    pub fn output(
        &mut self,
        context: &HarnessRunContext,
        output: AgentOutputChunk,
    ) -> Vec<HarnessEvent> {
        match self {
            Self::ClaudeCode(events) => events.output(context, output),
        }
    }

    pub fn complete(&self, context: &HarnessRunContext) -> Vec<HarnessEvent> {
        match self {
            Self::ClaudeCode(events) => events.complete(context),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HarnessRunContext {
    pub run_id: String,
    pub session_id: String,
    pub message_id: String,
    pub part_id: String,
}

impl HarnessRunContext {
    pub fn new(run_id: &str) -> Self {
        Self::for_session(run_id, run_id)
    }

    pub fn for_session(run_id: &str, session_id: &str) -> Self {
        Self {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
            message_id: run_id.to_owned(),
            part_id: format!("{run_id}_text"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HarnessEvent {
    pub event: &'static str,
    pub data: Value,
}

impl HarnessEvent {
    pub fn for_context(event: &'static str, context: &HarnessRunContext, mut data: Value) -> Self {
        if let Some(payload) = data.as_object_mut() {
            payload
                .entry("sessionID".to_owned())
                .or_insert_with(|| context.session_id.clone().into());
        }
        Self { event, data }
    }
}

fn is_stdout(stream: AgentOutputStreamKind) -> bool {
    matches!(stream, AgentOutputStreamKind::Stdout)
}

pub fn is_supported(harness: &str) -> bool {
    matches!(harness, claude_code::ID)
}

pub fn build_harness_run(
    agent: &AgentDefinition,
    prompt: &str,
) -> Result<HarnessRunSpec, GatewayError> {
    match agent.resolved_harness() {
        claude_code::ID => Ok(claude_code::build_run(agent, prompt)),
        harness => Err(GatewayError::InvalidConfig(format!(
            "unsupported harness: {harness}"
        ))),
    }
}
