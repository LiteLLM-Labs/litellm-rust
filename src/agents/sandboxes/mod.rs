pub mod e2b;

use futures_util::{stream::BoxStream, StreamExt};
use reqwest::Client;

use crate::{
    agents::sandboxes::e2b::E2bSandboxClient, errors::GatewayError, proxy::config::GeneralSettings,
};

pub type AgentOutputStream = BoxStream<'static, Result<AgentOutputChunk, GatewayError>>;

pub fn default_provider() -> &'static str {
    e2b::PROVIDER
}

pub fn is_supported_provider(provider: &str) -> bool {
    matches!(provider, e2b::PROVIDER)
}

#[derive(Debug, Clone)]
pub struct AgentOutputChunk {
    pub stream: AgentOutputStreamKind,
    pub delta: String,
}

impl AgentOutputChunk {
    pub fn stdout(delta: String) -> Self {
        Self {
            stream: AgentOutputStreamKind::Stdout,
            delta,
        }
    }

    pub fn stderr(delta: String) -> Self {
        Self {
            stream: AgentOutputStreamKind::Stderr,
            delta,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AgentOutputStreamKind {
    Stdout,
    Stderr,
}

impl AgentOutputStreamKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SandboxCommand {
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct SandboxSession {
    pub provider: &'static str,
    pub target_kind: ExecutionTargetKind,
    pub sandbox_id: Option<String>,
    target: SandboxTarget,
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutionTargetKind {
    Sandbox,
    Server,
}

impl ExecutionTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Server => "server",
        }
    }
}

#[derive(Debug, Clone)]
enum SandboxTarget {
    E2b(e2b::E2bSandbox),
}

#[derive(Debug, Clone)]
pub enum SandboxRunner {
    E2b(E2bSandboxClient),
}

impl SandboxRunner {
    pub fn from_settings(http: Client, settings: &GeneralSettings) -> Result<Self, GatewayError> {
        match settings
            .sandbox_choice
            .as_deref()
            .unwrap_or(default_provider())
        {
            e2b::PROVIDER => Ok(Self::E2b(E2bSandboxClient::new(
                http,
                settings.e2b_sandbox_params.clone(),
            ))),
            provider => Err(GatewayError::InvalidConfig(format!(
                "unsupported sandbox_choice: {provider}"
            ))),
        }
    }

    pub async fn create(&self, run_id: &str) -> Result<SandboxSession, GatewayError> {
        match self {
            Self::E2b(client) => {
                let sandbox = client.create(run_id).await?;
                Ok(SandboxSession {
                    provider: e2b::PROVIDER,
                    target_kind: ExecutionTargetKind::Sandbox,
                    sandbox_id: Some(sandbox.id.clone()),
                    target: SandboxTarget::E2b(sandbox),
                })
            }
        }
    }

    pub async fn start(
        &self,
        session: &SandboxSession,
        command: SandboxCommand,
    ) -> Result<AgentOutputStream, GatewayError> {
        match (self, &session.target) {
            (Self::E2b(client), SandboxTarget::E2b(sandbox)) => {
                client.start_command(sandbox, command).await
            }
        }
    }

    pub async fn terminate(&self, session: &SandboxSession) -> Result<(), GatewayError> {
        match (self, &session.target) {
            (Self::E2b(client), SandboxTarget::E2b(sandbox)) => client.terminate(&sandbox.id).await,
        }
    }
}

pub(crate) fn boxed_stream<S>(stream: S) -> AgentOutputStream
where
    S: futures_util::Stream<Item = Result<AgentOutputChunk, GatewayError>> + Send + 'static,
{
    stream.boxed()
}
