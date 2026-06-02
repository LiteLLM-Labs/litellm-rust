use serde::Deserialize;
use serde_json::json;

use crate::agents::{
    config::AgentDefinition,
    events,
    harnesses::{is_stdout, HarnessEvent, HarnessEvents, HarnessRunContext, HarnessRunSpec},
    sandboxes::AgentOutputChunk,
};

pub const ID: &str = "claude-code";

pub fn build_run(agent: &AgentDefinition, prompt: &str) -> HarnessRunSpec {
    HarnessRunSpec {
        command: format!(
            "set -euo pipefail\nnpm install --silent --no-audit --no-fund @anthropic-ai/claude-agent-sdk@latest >/dev/null\nLITELLM_AGENT_PROMPT={} LITELLM_AGENT_MODEL={} LITELLM_AGENT_SYSTEM={} node --input-type=module <<'LITELLM_CLAUDE_AGENT_SDK'\n{}\nLITELLM_CLAUDE_AGENT_SDK",
            shell_quote(prompt),
            shell_quote(&agent.model),
            shell_quote(&agent.system),
            CLAUDE_AGENT_SDK_SCRIPT
        ),
        events: HarnessEvents::ClaudeCode(ClaudeCodeEvents::default()),
    }
}

const CLAUDE_AGENT_SDK_SCRIPT: &str = r#"import { query } from "@anthropic-ai/claude-agent-sdk";

const prompt = process.env.LITELLM_AGENT_PROMPT ?? "";
const model = process.env.LITELLM_AGENT_MODEL || undefined;
const append = process.env.LITELLM_AGENT_SYSTEM || undefined;
const options = {
  includePartialMessages: true,
  permissionMode: "bypassPermissions",
  allowDangerouslySkipPermissions: true,
  systemPrompt: append
    ? { type: "preset", preset: "claude_code", append }
    : { type: "preset", preset: "claude_code" },
  ...(model ? { model } : {}),
};

for await (const message of query({ prompt, options })) {
  if (message.type !== "stream_event") continue;
  const event = message.event;
  if (event?.type !== "content_block_delta") continue;
  const delta = event.delta;
  if (delta?.type === "text_delta" && delta.text) {
    process.stdout.write(JSON.stringify({ type: "text_delta", text: delta.text }) + "\n");
  }
}
"#;

#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeEvents {
    stdout_buffer: String,
}

impl ClaudeCodeEvents {
    pub fn start(&self, context: &HarnessRunContext) -> Vec<HarnessEvent> {
        vec![
            HarnessEvent::new(
                events::SESSION_STATUS,
                json!({ "status": { "type": "busy" } }),
            ),
            HarnessEvent::new(
                events::MESSAGE_UPDATED,
                json!({
                    "info": {
                        "id": context.message_id,
                        "role": "assistant",
                        "sessionID": context.run_id,
                    }
                }),
            ),
            HarnessEvent::new(
                events::MESSAGE_PART_UPDATED,
                json!({
                    "part": {
                        "id": context.part_id,
                        "messageID": context.message_id,
                        "sessionID": context.run_id,
                        "type": "text",
                        "text": "",
                    }
                }),
            ),
        ]
    }

    pub fn output(
        &mut self,
        context: &HarnessRunContext,
        output: AgentOutputChunk,
    ) -> Vec<HarnessEvent> {
        if output.delta.is_empty() || !is_stdout(output.stream) {
            return Vec::new();
        }

        self.stdout_buffer.push_str(&output.delta);
        let mut events = Vec::new();
        while let Some(newline) = self.stdout_buffer.find('\n') {
            let line = self.stdout_buffer[..newline].to_owned();
            self.stdout_buffer.drain(..=newline);
            let Ok(delta) = serde_json::from_str::<ClaudeCodeOutput>(&line) else {
                continue;
            };
            match delta {
                ClaudeCodeOutput::TextDelta { text } if !text.is_empty() => {
                    events.push(HarnessEvent::new(
                        events::MESSAGE_PART_DELTA,
                        json!({
                            "messageID": context.message_id,
                            "partID": context.part_id,
                            "field": "text",
                            "delta": text,
                        }),
                    ));
                }
                _ => {}
            }
        }
        events
    }

    pub fn complete(&self, context: &HarnessRunContext) -> Vec<HarnessEvent> {
        vec![
            HarnessEvent::new(
                events::MESSAGE_UPDATED,
                json!({
                    "info": {
                        "id": context.message_id,
                        "role": "assistant",
                        "finish": "stop",
                        "sessionID": context.run_id,
                    }
                }),
            ),
            HarnessEvent::new(events::SESSION_IDLE, json!({ "sessionID": context.run_id })),
        ]
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeCodeOutput {
    TextDelta { text: String },
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
