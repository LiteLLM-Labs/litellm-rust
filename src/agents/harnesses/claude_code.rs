use crate::agents::{config::AgentDefinition, harnesses::HarnessRunSpec};

pub const ID: &str = "claude-code";

pub fn build_run(agent: &AgentDefinition, prompt: &str) -> HarnessRunSpec {
    HarnessRunSpec {
        command: format!(
            "claude -p {} --model {} --append-system-prompt {}",
            shell_quote(prompt),
            shell_quote(&agent.model),
            shell_quote(&agent.system)
        ),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
