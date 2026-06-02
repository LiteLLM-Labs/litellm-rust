use axum::http::{header, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    providers::{
        router::Deployment,
        transform::{ProviderRequest, Transformation},
    },
};

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Default, Clone)]
pub struct AnthropicTransformation;

impl Transformation for AnthropicTransformation {
    fn transform_request(
        &self,
        mut body: Value,
        deployment: &Deployment,
        inbound_headers: &HeaderMap,
    ) -> Result<ProviderRequest, GatewayError> {
        if body.get("model").and_then(Value::as_str) != Some(deployment.upstream_model.as_str()) {
            body["model"] = Value::String(deployment.upstream_model.clone());
        }
        let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&deployment.api_key)
                .map_err(|_| GatewayError::InvalidConfig("invalid api_key".to_owned()))?,
        );
        headers.insert(
            "anthropic-version",
            inbound_headers
                .get("anthropic-version")
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION)),
        );
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        if let Some(beta) = inbound_headers.get("anthropic-beta") {
            headers.insert("anthropic-beta", beta.clone());
        }

        Ok(ProviderRequest {
            body: serde_json::to_vec(&body)?,
            headers,
            stream,
        })
    }

    fn transform_response_headers(&self, upstream: &HeaderMap, stream: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let content_type = if stream {
            HeaderValue::from_static("text/event-stream")
        } else {
            upstream
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("application/json"))
        };
        headers.insert(header::CONTENT_TYPE, content_type);
        if let Some(request_id) = upstream.get("request-id").cloned() {
            headers.insert("request-id", request_id);
        }
        headers
    }
}
