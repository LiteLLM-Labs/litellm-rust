use axum::http::HeaderMap;
use serde_json::Value;

use crate::{app::errors::GatewayError, providers::deployment::Deployment};

#[derive(Debug)]
pub struct ProviderRequest {
    pub body: Vec<u8>,
    pub stream: bool,
}

pub trait MessagesTransformation: Send + Sync + 'static {
    fn transform_request(
        &self,
        body: Value,
        deployment: &Deployment,
    ) -> Result<ProviderRequest, GatewayError>;

    fn transform_response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap;
}
