use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("config read failed: {0}")]
    ConfigRead(#[from] std::io::Error),

    #[error("config parse failed: {0}")]
    ConfigParse(#[from] serde_yaml::Error),

    #[error("http client init failed: {0}")]
    HttpClient(reqwest::Error),

    #[error("invalid request json: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("invalid request json: {0}")]
    InvalidJsonMessage(String),

    #[error("database is not configured")]
    MissingDatabase,

    #[error("database request failed: {0}")]
    Database(sqlx::Error),

    #[error("database migration failed: {0}")]
    Migration(sqlx::migrate::MigrateError),

    #[error("missing model")]
    MissingModel,

    #[error("unknown model: {0}")]
    UnknownModel(String),

    #[error("mcp server selection is required")]
    MissingMcpServer,

    #[error("unknown mcp server: {0}")]
    UnknownMcpServer(String),

    #[error("{0}")]
    NotFound(String),

    #[error("credential encryption failed: {0}")]
    Crypto(String),

    #[error("no stored credential for mcp server '{0}'; set one via POST /v1/mcp/server/{0}/user-credential")]
    UserCredentialMissing(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("upstream request failed: {0}")]
    Upstream(reqwest::Error),
}

impl GatewayError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidConfig(_)
            | Self::ConfigRead(_)
            | Self::ConfigParse(_)
            | Self::HttpClient(_)
            | Self::Database(_)
            | Self::Crypto(_)
            | Self::Migration(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::MissingDatabase => StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidJson(_)
            | Self::InvalidJsonMessage(_)
            | Self::MissingModel
            | Self::MissingMcpServer => StatusCode::BAD_REQUEST,
            Self::UnknownModel(_) | Self::UnknownMcpServer(_) | Self::NotFound(_) => {
                StatusCode::NOT_FOUND
            }
            Self::Unauthorized | Self::UserCredentialMissing(_) => StatusCode::UNAUTHORIZED,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(json!({
            "error": {
                "type": "gateway_error",
                "message": self.to_string()
            }
        }));
        (status, body).into_response()
    }
}
