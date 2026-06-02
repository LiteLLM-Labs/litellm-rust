use bytes::Bytes;
use futures_util::{stream, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    agents::{
        config::E2bSandboxParams,
        sandboxes::{boxed_stream, AgentOutputChunk, AgentOutputStream, SandboxCommand},
    },
    errors::GatewayError,
};

pub const PROVIDER: &str = "e2b";

#[derive(Debug, Clone)]
pub struct E2bSandboxClient {
    http: Client,
    settings: E2bSandboxParams,
}

#[derive(Debug, Clone)]
pub struct E2bSandbox {
    pub id: String,
    access_token: String,
    process_base_url: String,
}

impl E2bSandboxClient {
    pub fn new(http: Client, settings: E2bSandboxParams) -> Self {
        Self { http, settings }
    }

    pub async fn create(&self, run_id: &str) -> Result<E2bSandbox, GatewayError> {
        let api_key = self.api_key()?;
        let response = self
            .http
            .post(format!(
                "{}/sandboxes",
                self.settings.e2b_api_base.trim_end_matches('/')
            ))
            .header("X-API-Key", api_key)
            .json(&CreateSandboxRequest {
                template_id: &self.settings.e2b_template,
                timeout: self.settings.timeout_seconds,
                secure: true,
                allow_internet_access: true,
                metadata: SandboxMetadata { run_id },
            })
            .send()
            .await
            .map_err(GatewayError::Sandbox)?;

        if !response.status().is_success() {
            return Err(GatewayError::SandboxError(format!(
                "E2B create sandbox failed with status {}",
                response.status()
            )));
        }

        let created: CreateSandboxResponse =
            response.json().await.map_err(GatewayError::Sandbox)?;
        let access_token = created.envd_access_token.ok_or_else(|| {
            GatewayError::SandboxError(
                "E2B create sandbox did not return envdAccessToken".to_owned(),
            )
        })?;
        let process_base_url = created
            .domain
            .filter(|domain| !domain.trim().is_empty())
            .unwrap_or_else(|| format!("https://49999-{}.e2b.app", created.sandbox_id));

        Ok(E2bSandbox {
            id: created.sandbox_id,
            access_token,
            process_base_url,
        })
    }

    pub async fn start_command(
        &self,
        sandbox: &E2bSandbox,
        command: SandboxCommand,
    ) -> Result<AgentOutputStream, GatewayError> {
        let response = self
            .http
            .post(format!(
                "{}/process.Process/Start",
                sandbox.process_base_url.trim_end_matches('/')
            ))
            .header("X-Access-Token", &sandbox.access_token)
            .header("Authorization", "Basic dXNlcjo=")
            .header("Connect-Protocol-Version", "1")
            .header("Content-Type", "application/connect+json")
            .json(&StartProcessRequest {
                process: StartProcess {
                    cmd: "bash",
                    args: vec!["-lc", &command.command],
                    cwd: &self.settings.workspace_dir,
                },
                stdin: false,
            })
            .send()
            .await
            .map_err(GatewayError::Sandbox)?;

        if !response.status().is_success() {
            return Err(GatewayError::SandboxError(format!(
                "E2B start process failed with status {}",
                response.status()
            )));
        }

        let stream = response
            .bytes_stream()
            .map_err(GatewayError::Sandbox)
            .map_ok(|bytes| stream::iter(decode_process_chunk(bytes).into_iter().map(Ok)))
            .try_flatten();

        Ok(boxed_stream(stream))
    }

    pub async fn terminate(&self, sandbox_id: &str) -> Result<(), GatewayError> {
        let api_key = self.api_key()?;
        let response = self
            .http
            .delete(format!(
                "{}/sandboxes/{}",
                self.settings.e2b_api_base.trim_end_matches('/'),
                sandbox_id
            ))
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(GatewayError::Sandbox)?;

        if !response.status().is_success() {
            return Err(GatewayError::SandboxError(format!(
                "E2B terminate sandbox failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }

    fn api_key(&self) -> Result<&str, GatewayError> {
        self.settings
            .e2b_api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::InvalidConfig(
                    "general_settings.e2b_sandbox_params.e2b_api_key is required".to_owned(),
                )
            })
    }
}

fn decode_process_chunk(bytes: Bytes) -> Vec<AgentOutputChunk> {
    if bytes.is_empty() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut chunks = Vec::new();

    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        collect_output_chunks(&value, &mut chunks);
    } else {
        for line in text.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                collect_output_chunks(&value, &mut chunks);
            }
        }
    }

    if chunks.is_empty() {
        chunks.push(AgentOutputChunk::stdout(text.into_owned()));
    }

    chunks
}

fn collect_output_chunks(value: &Value, chunks: &mut Vec<AgentOutputChunk>) {
    let Some(object) = value.as_object() else {
        return;
    };

    let mut found = false;
    if let Some(delta) = object.get("stdout").and_then(Value::as_str) {
        chunks.push(AgentOutputChunk::stdout(delta.to_owned()));
        found = true;
    }
    if let Some(delta) = object.get("stderr").and_then(Value::as_str) {
        chunks.push(AgentOutputChunk::stderr(delta.to_owned()));
        found = true;
    }
    if found {
        return;
    }

    for key in ["output", "text", "message"] {
        if let Some(delta) = object.get(key).and_then(Value::as_str) {
            chunks.push(AgentOutputChunk::stdout(delta.to_owned()));
            return;
        }
    }

    for nested in object.values() {
        collect_output_chunks(nested, chunks);
    }
}

#[derive(Serialize)]
struct CreateSandboxRequest<'a> {
    #[serde(rename = "templateID")]
    template_id: &'a str,
    timeout: u64,
    secure: bool,
    allow_internet_access: bool,
    metadata: SandboxMetadata<'a>,
}

#[derive(Serialize)]
struct SandboxMetadata<'a> {
    run_id: &'a str,
}

#[derive(Deserialize)]
struct CreateSandboxResponse {
    #[serde(rename = "sandboxID")]
    sandbox_id: String,
    #[serde(rename = "envdAccessToken")]
    envd_access_token: Option<String>,
    domain: Option<String>,
}

#[derive(Serialize)]
struct StartProcessRequest<'a> {
    process: StartProcess<'a>,
    stdin: bool,
}

#[derive(Serialize)]
struct StartProcess<'a> {
    cmd: &'a str,
    args: Vec<&'a str>,
    cwd: &'a str,
}
