use axum::http::{header, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::{
    app::errors::GatewayError,
    providers::{
        base::{MessagesTransformation, ProviderRequest},
        deployment::Deployment,
    },
};

#[derive(Debug, Default, Clone)]
pub struct AnthropicMessagesTransformation;

impl MessagesTransformation for AnthropicMessagesTransformation {
    fn transform_request(
        &self,
        mut body: Value,
        deployment: &Deployment,
    ) -> Result<ProviderRequest, GatewayError> {
        if requested_model(&body)? != deployment.upstream_model {
            body["model"] = Value::String(deployment.upstream_model.clone());
        }

        let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
        Ok(ProviderRequest {
            body: serde_json::to_vec(&body)?,
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

pub fn parse_body(raw: &[u8]) -> Result<Value, GatewayError> {
    serde_json::from_slice(raw).map_err(GatewayError::InvalidJson)
}

pub fn requested_model(body: &Value) -> Result<&str, GatewayError> {
    body.get("model")
        .and_then(Value::as_str)
        .ok_or(GatewayError::MissingModel)
}
