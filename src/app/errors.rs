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

    #[error("missing model")]
    MissingModel,

    #[error("unknown model: {0}")]
    UnknownModel(String),

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
            | Self::HttpClient(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::InvalidJson(_) | Self::MissingModel => StatusCode::BAD_REQUEST,
            Self::UnknownModel(_) => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
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
