use serde::{Deserialize, Serialize};

/// Body for `POST /v1/mcp/server/{server_id}/user-credential`.
#[derive(Debug, Deserialize)]
pub struct McpUserCredentialRequest {
    pub credential: String,
    #[serde(default = "default_true")]
    pub save: bool,
}

/// Body for `POST /v1/mcp/server/{server_id}/oauth-user-credential`.
#[derive(Debug, Deserialize)]
pub struct McpOAuthUserCredentialRequest {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    pub scopes: Option<Vec<String>>,
}

/// Per-server status row returned by the credential status + list endpoints.
#[derive(Debug, Serialize)]
pub struct McpUserCredentialStatus {
    pub server_id: String,
    pub credential_type: String,
    pub has_credential: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// A decrypted credential resolved for request-time injection.
#[derive(Debug)]
pub struct ResolvedCredential {
    /// The token value to inject (static token or OAuth access token).
    pub value: String,
}

fn default_true() -> bool {
    true
}
