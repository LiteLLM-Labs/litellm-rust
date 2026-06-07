//! The only place that does outbound networking to providers.

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use reqwest::{Client, Response as UpstreamResponse};

use crate::errors::GatewayError;

pub async fn send_request(
    http: &Client,
    url: String,
    body: Vec<u8>,
    headers: HeaderMap,
) -> Result<UpstreamResponse, GatewayError> {
    let mut req = http.post(url).body(body);
    for (name, value) in &headers {
        req = req.header(name, value);
    }
    req.send().await.map_err(GatewayError::Upstream)
}

/// Pass the upstream response through byte-for-byte (fast path: inbound wire ==
/// outbound wire).
pub async fn build_response(upstream: UpstreamResponse, headers: HeaderMap) -> Response {
    let status = upstream.status();
    let body_stream = upstream.bytes_stream().map_err(std::io::Error::other);
    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

/// Build a response from a transformed byte stream (cross-protocol streaming).
pub fn build_stream_response<S>(status: StatusCode, headers: HeaderMap, stream: S) -> Response
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

/// Build a non-streaming response from already-serialized bytes.
pub fn build_bytes_response(status: StatusCode, headers: HeaderMap, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
