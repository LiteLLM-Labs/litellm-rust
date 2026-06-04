use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    sdk::{
        providers::transform::{ProviderRequest, Transformation},
        router::Deployment,
    },
};

// Headers Codex attaches to each turn. Forwarded so upstream logging/analytics
// keep request correlation; harmless to OpenAI if it ignores them.
const FORWARDED_HEADERS: &[&str] = &[
    "accept",
    "originator",
    "session-id",
    "thread-id",
    "x-client-request-id",
    "x-codex-beta-features",
    "x-codex-turn-metadata",
    "x-codex-window-id",
];

#[derive(Debug, Default, Clone)]
pub struct OpenAiResponsesTransformation;

impl Transformation for OpenAiResponsesTransformation {
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
        let bearer = format!("Bearer {}", deployment.api_key);
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&bearer)
                .map_err(|_| GatewayError::InvalidConfig("invalid api_key".to_owned()))?,
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for name in FORWARDED_HEADERS {
            if let Some(value) = inbound_headers.get(*name) {
                if let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) {
                    headers.insert(header_name, value.clone());
                }
            }
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
        if let Some(request_id) = upstream.get("x-request-id").cloned() {
            headers.insert("x-request-id", request_id);
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{header, HeaderMap, HeaderValue};
    use serde_json::json;

    use super::OpenAiResponsesTransformation;
    use crate::sdk::{providers::transform::Transformation, router::Deployment};

    fn deployment() -> Deployment {
        Deployment {
            provider_id: "openai".to_owned(),
            upstream_model: "gpt-5.5".to_owned(),
            api_base: "https://api.openai.com".to_owned(),
            api_key: "sk-upstream".to_owned(),
        }
    }

    #[test]
    fn rewrites_model_and_sets_bearer_auth() {
        let req = OpenAiResponsesTransformation
            .transform_request(
                json!({ "model": "gpt-codex", "input": [] }),
                &deployment(),
                &HeaderMap::new(),
            )
            .unwrap();

        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(
            req.headers.get(header::AUTHORIZATION).unwrap(),
            "Bearer sk-upstream"
        );
        assert!(!req.stream);
    }

    #[test]
    fn detects_stream_flag() {
        let req = OpenAiResponsesTransformation
            .transform_request(
                json!({ "model": "gpt-5.5", "stream": true }),
                &deployment(),
                &HeaderMap::new(),
            )
            .unwrap();
        assert!(req.stream);
    }

    #[test]
    fn forwards_codex_headers() {
        let mut inbound = HeaderMap::new();
        inbound.insert("originator", HeaderValue::from_static("codex_exec"));
        inbound.insert("session-id", HeaderValue::from_static("abc"));

        let req = OpenAiResponsesTransformation
            .transform_request(json!({ "model": "gpt-5.5" }), &deployment(), &inbound)
            .unwrap();

        assert_eq!(req.headers.get("originator").unwrap(), "codex_exec");
        assert_eq!(req.headers.get("session-id").unwrap(), "abc");
    }

    #[test]
    fn streaming_response_is_event_stream() {
        let headers =
            OpenAiResponsesTransformation.transform_response_headers(&HeaderMap::new(), true);
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "text/event-stream");
    }
}
