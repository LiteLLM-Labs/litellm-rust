//! Request pipeline: translate inbound → IR → outbound, send upstream, then
//! translate the response back. When inbound and outbound wire formats match,
//! the body is passed through untouched (fast path), preserving low overhead.

mod dispatch;
mod respond;
mod transform;

use std::sync::Arc;

use axum::{http::HeaderMap, response::Response};
use serde_json::Value;

use crate::{
    errors::GatewayError, http::credential_overrides, proxy::state::AppState,
    sdk::codec::WireFormat,
};

/// Drive one request through the gateway. `model` is the public model name (from
/// the body or, for Gemini, the URL path); `stream` is the resolved streaming
/// flag for the request.
pub async fn handle(
    state: &Arc<AppState>,
    inbound_wire: WireFormat,
    model: String,
    stream: bool,
    body: Value,
    inbound_headers: &HeaderMap,
) -> Result<Response, GatewayError> {
    let route =
        credential_overrides::apply(state, state.router.resolve_wire(inbound_wire, &model)?)
            .await?;
    let deployment = route.deployment;
    let out_wire = deployment.wire;
    let url = deployment.upstream_url(stream);

    if inbound_wire == out_wire {
        dispatch::run_fast_path(state, &deployment, url, stream, body, inbound_headers).await
    } else {
        dispatch::run_cross_protocol(
            state,
            inbound_wire,
            out_wire,
            &deployment,
            url,
            model,
            stream,
            body,
            inbound_headers,
        )
        .await
    }
}
