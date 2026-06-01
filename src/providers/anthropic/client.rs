use axum::http::HeaderMap;
use reqwest::{Client, Response};

use crate::{
    app::errors::GatewayError,
    models::deployment::Deployment,
    providers::{anthropic::headers::apply_headers, base::ProviderRequest},
};

pub async fn send_messages(
    http: &Client,
    inbound_headers: &HeaderMap,
    deployment: &Deployment,
    prepared: ProviderRequest,
) -> Result<Response, GatewayError> {
    let request = http.post(deployment.messages_url()).body(prepared.body);
    let request = apply_headers(request, inbound_headers, deployment);
    request.send().await.map_err(GatewayError::Upstream)
}
