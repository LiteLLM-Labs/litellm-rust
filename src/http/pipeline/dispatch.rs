//! Same-protocol fast path and cross-protocol translation dispatch.

use std::sync::Arc;

use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    http::llm,
    proxy::state::AppState,
    sdk::codec::{codec_for, ProtocolCodec, RequestCtx, WireFormat},
};

use super::respond::{
    error_or_passthrough, has_error_object, is_event_stream, is_failed_responses, rewrite_model,
    translated_error,
};
use super::transform::transform_stream;

/// Same protocol both sides: rewrite the model and pass the body through.
pub(super) async fn run_fast_path(
    state: &Arc<AppState>,
    deployment: &crate::sdk::router::Deployment,
    url: String,
    stream: bool,
    mut body: Value,
    inbound_headers: &HeaderMap,
) -> Result<Response, GatewayError> {
    let out_codec = codec_for(deployment.wire);
    rewrite_model(&mut body, deployment);
    let headers = out_codec.outbound_headers(deployment, inbound_headers)?;
    let upstream = llm::send_request(&state.http, url, serde_json::to_vec(&body)?, headers).await?;
    let status = upstream.status();
    if stream {
        return fast_path_stream(out_codec, upstream).await;
    }
    let resp_headers = out_codec.response_headers(upstream.headers(), false);
    let bytes = upstream.bytes().await.map_err(GatewayError::Upstream)?;
    // A same-wire Responses upstream can return a bare 200 `{error}`; normalize it to
    // the protocol's failed envelope (its other codecs already produce that shape).
    if deployment.wire == WireFormat::OpenAiResponses
        && has_error_object(&bytes)
        && !is_failed_responses(&bytes)
    {
        let ctx = RequestCtx {
            model: deployment.upstream_model.clone(),
            stream: false,
        };
        return translated_error(out_codec, &ctx, status, resp_headers, &bytes);
    }
    Ok(llm::build_bytes_response(
        status,
        resp_headers,
        bytes.to_vec(),
    ))
}

/// Same-protocol streaming: pass the upstream stream through. The SSE content type
/// is only set for a genuine event stream (a 200 JSON body on a streaming request
/// is an error and keeps its JSON type).
async fn fast_path_stream(
    out_codec: &dyn ProtocolCodec,
    upstream: reqwest::Response,
) -> Result<Response, GatewayError> {
    let status = upstream.status();
    let sse = status.is_success() && is_event_stream(upstream.headers());
    let resp_headers = out_codec.response_headers(upstream.headers(), sse);
    Ok(llm::build_response(upstream, resp_headers).await)
}

/// Cross-protocol: parse to IR, render to the outbound wire, then translate the
/// response (or stream) back to the inbound protocol.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_cross_protocol(
    state: &Arc<AppState>,
    inbound_wire: WireFormat,
    out_wire: WireFormat,
    deployment: &crate::sdk::router::Deployment,
    url: String,
    model: String,
    stream: bool,
    body: Value,
    inbound_headers: &HeaderMap,
) -> Result<Response, GatewayError> {
    let in_codec = codec_for(inbound_wire);
    let out_codec = codec_for(out_wire);
    let ctx = RequestCtx {
        model: model.clone(),
        stream,
    };

    let mut ir_req = in_codec.parse_request(body)?;
    ir_req.model = deployment.upstream_model.clone();
    ir_req.stream = stream;
    let out_body = out_codec.render_request(&ir_req)?;
    let headers = out_codec.outbound_headers(deployment, inbound_headers)?;
    let upstream =
        llm::send_request(&state.http, url, serde_json::to_vec(&out_body)?, headers).await?;

    let status = upstream.status();
    let resp_headers = in_codec.response_headers(upstream.headers(), false);
    // A non-2xx, or a 200 non-SSE JSON body on a streaming request, is an error: read
    // it and render the inbound protocol's error shape (or pass a non-error through).
    if !status.is_success() || (stream && !is_event_stream(upstream.headers())) {
        let bytes = upstream.bytes().await.map_err(GatewayError::Upstream)?;
        return error_or_passthrough(in_codec, &ctx, status, resp_headers, &bytes);
    }

    if stream {
        return stream_cross_protocol(in_codec, out_codec, &ctx, upstream);
    }

    let bytes = upstream.bytes().await.map_err(GatewayError::Upstream)?;
    // A 200 body with a top-level error translates to the inbound error shape. A
    // Responses `status:"failed"` is left to its codec (it maps to an IR error).
    if has_error_object(&bytes) && !is_failed_responses(&bytes) {
        return translated_error(in_codec, &ctx, status, resp_headers, &bytes);
    }
    let upstream_json: Value = serde_json::from_slice(&bytes).map_err(GatewayError::InvalidJson)?;
    let ir_resp = out_codec.parse_response(upstream_json)?;
    log_usage(state, deployment, &ir_resp.usage);
    let client_value = in_codec.render_response(&ir_resp, &ctx)?;
    let out_bytes = serde_json::to_vec(&client_value)?;
    Ok(llm::build_bytes_response(status, resp_headers, out_bytes))
}

/// Bridge an upstream SSE stream back to the inbound protocol.
fn stream_cross_protocol(
    in_codec: &dyn ProtocolCodec,
    out_codec: &dyn ProtocolCodec,
    ctx: &RequestCtx,
    upstream: reqwest::Response,
) -> Result<Response, GatewayError> {
    let resp_headers = in_codec.response_headers(upstream.headers(), true);
    let parser = out_codec.stream_parser();
    let renderer = in_codec.stream_renderer(ctx);
    let body_stream = transform_stream(upstream, parser, renderer);
    Ok(llm::build_stream_response(
        StatusCode::OK,
        resp_headers,
        body_stream,
    ))
}

/// Emit the per-request usage/cost tracing line.
fn log_usage(
    state: &Arc<AppState>,
    deployment: &crate::sdk::router::Deployment,
    usage: &crate::sdk::codec::ir::Usage,
) {
    let cost = state
        .model_cost_map
        .get(&deployment.upstream_model)
        .and_then(|info| info.compute_cost(usage));
    tracing::info!(
        model = %deployment.upstream_model,
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        cache_read_tokens = usage.cache_read_input_tokens,
        cache_creation_tokens = usage.cache_creation_input_tokens,
        cost_usd = ?cost,
        "request usage"
    );
}
