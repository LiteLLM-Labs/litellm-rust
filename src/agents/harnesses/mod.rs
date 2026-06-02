pub mod claude_code;

use crate::{agents::config::AgentDefinition, errors::GatewayError};

#[derive(Debug, Clone)]
pub struct HarnessRunSpec {
    pub command: String,
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
