//! The only place that does outbound networking to providers.

use axum::{body::Body, http::HeaderMap, response::Response};
use futures_util::TryStreamExt;
use reqwest::{Client, Response as UpstreamResponse};

use crate::{app::errors::GatewayError, providers::transform::ProviderRequest};

pub async fn send_request(
    http: &Client,
    url: String,
    prepared: ProviderRequest,
) -> Result<UpstreamResponse, GatewayError> {
    let mut req = http.post(url).body(prepared.body);
    for (name, value) in &prepared.headers {
        req = req.header(name, value);
    }
    req.send().await.map_err(GatewayError::Upstream)
}

pub async fn build_response(upstream: UpstreamResponse, headers: HeaderMap) -> Response {
    let status = upstream.status();
    let body_stream = upstream.bytes_stream().map_err(std::io::Error::other);
    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
