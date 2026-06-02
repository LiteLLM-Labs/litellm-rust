use serde::{Deserialize, Serialize};

/// Request body for `POST /user/new` (subset of LiteLLM's `NewUserRequest`).
#[derive(Debug, Default, Deserialize)]
pub struct NewUserRequest {
    pub user_id: Option<String>,
    pub user_alias: Option<String>,
    #[serde(default = "default_true")]
    pub auto_create_key: bool,
    pub key_alias: Option<String>,
}

/// Request body for `POST /key/generate` (subset of LiteLLM's `GenerateKeyRequest`).
#[derive(Debug, Default, Deserialize)]
pub struct GenerateKeyRequest {
    pub user_id: Option<String>,
    pub key_alias: Option<String>,
}

/// Response returned when creating a user or key.
#[derive(Debug, Serialize)]
pub struct NewUserResponse {
    pub user_id: String,
    /// The generated API key, or `None` when `auto_create_key` was false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GenerateKeyResponse {
    pub user_id: String,
    pub key: String,
}

fn default_true() -> bool {
    true
}
