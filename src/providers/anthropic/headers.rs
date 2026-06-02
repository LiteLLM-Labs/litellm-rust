use axum::http::{HeaderMap, HeaderValue};
use reqwest::RequestBuilder;

use crate::providers::deployment::Deployment;

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

pub fn apply_headers(
    mut request: RequestBuilder,
    inbound: &HeaderMap,
    deployment: &Deployment,
) -> RequestBuilder {
    request = request
        .header("x-api-key", deployment.api_key.as_str())
        .header("anthropic-version", anthropic_version(inbound))
        .header("content-type", "application/json");

    if let Some(beta) = inbound.get("anthropic-beta") {
        request = request.header("anthropic-beta", beta.clone());
    }

    request
}

fn anthropic_version(inbound: &HeaderMap) -> HeaderValue {
    inbound
        .get("anthropic-version")
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION))
}
