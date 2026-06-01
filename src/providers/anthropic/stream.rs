use axum::{body::Body, http::HeaderMap, response::Response};
use futures_util::TryStreamExt;
use reqwest::Response as UpstreamResponse;

pub async fn response_from_upstream(upstream: UpstreamResponse, headers: HeaderMap) -> Response {
    let status = upstream.status();
    let body_stream = upstream.bytes_stream().map_err(std::io::Error::other);
    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
