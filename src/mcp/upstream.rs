use axum::{
    body::{Body, Bytes},
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderMap, HeaderName, HeaderValue, Method,
    },
    response::Response,
};
use futures_util::TryStreamExt;
use reqwest::Client;

use crate::{errors::GatewayError, mcp::registry::McpServer};

const MCP_SESSION_ID: &str = "mcp-session-id";
const MCP_PROTOCOL_VERSION: &str = "mcp-protocol-version";

pub async fn forward_streamable_http(
    http: &Client,
    server: &McpServer,
    method: Method,
    inbound_headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    let mut request = http.request(method, server.url.clone());

    for (name, value) in request_headers(inbound_headers) {
        request = request.header(name, value);
    }
    for (name, value) in &server.headers {
        request = request.header(name, value);
    }
    if let Some(api_key) = server.api_key.as_deref() {
        request = request.bearer_auth(api_key);
    }
    if !body.is_empty() {
        request = request.body(body);
    }

    let upstream = request.send().await.map_err(GatewayError::Upstream)?;
    let status = upstream.status();
    let headers = response_headers(upstream.headers());
    let body_stream = upstream.bytes_stream().map_err(std::io::Error::other);
    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;

    if !response.headers().contains_key(CONTENT_TYPE) {
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    Ok(response)
}

fn request_headers(headers: &HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
    [ACCEPT, CONTENT_TYPE]
        .into_iter()
        .chain(mcp_header_names())
        .filter_map(|name| headers.get(&name).map(|value| (name, value.clone())))
        .collect()
}

fn response_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut copied = HeaderMap::new();
    for name in [CONTENT_TYPE].into_iter().chain(mcp_header_names()) {
        if let Some(value) = headers
            .get(name.as_str())
            .and_then(|value| HeaderValue::from_bytes(value.as_bytes()).ok())
        {
            copied.insert(name, value);
        }
    }
    copied
}

fn mcp_header_names() -> [HeaderName; 2] {
    [
        HeaderName::from_static(MCP_SESSION_ID),
        HeaderName::from_static(MCP_PROTOCOL_VERSION),
    ]
}
