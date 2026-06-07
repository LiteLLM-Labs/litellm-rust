//! Same-protocol fast path and cross-protocol translation dispatch.

use std::sync::Arc;

use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::TryStreamExt;
use serde_json::Value;

use crate::{
    errors::GatewayError,
    http::llm,
    proxy::state::AppState,
    sdk::codec::{codec_for, ir::StopReason, ProtocolCodec, RequestCtx, WireFormat},
};

use super::cache::{content_type_of, store_response, tee_and_store};
use super::respond::{
    error_or_passthrough, has_error_object, is_event_stream, is_failed_responses, rewrite_model,
    translated_error,
};
use super::transform::transform_stream;
use super::CachePlan;

/// Same protocol both sides: rewrite the model and pass the body through,
/// optionally caching the response.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_fast_path(
    state: &Arc<AppState>,
    deployment: &crate::sdk::router::Deployment,
    url: String,
    stream: bool,
    mut body: Value,
    inbound_headers: &HeaderMap,
    plan: &CachePlan,
) -> Result<Response, GatewayError> {
    let out_codec = codec_for(deployment.wire);
    rewrite_model(&mut body, deployment);
    let headers = out_codec.outbound_headers(deployment, inbound_headers)?;
    let upstream = llm::send_request(&state.http, url, serde_json::to_vec(&body)?, headers).await?;
    let status = upstream.status();
    if stream {
        return fast_path_stream(state, out_codec, upstream, status, plan).await;
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
    // Only cache a genuine success (no error body / failed status).
    let failed = !status.is_success() || is_failed_responses(&bytes) || has_error_object(&bytes);
    if !failed && (plan.store_key.is_some() || plan.semantic_text.is_some()) {
        store_response(
            state,
            plan.store_key.clone(),
            plan.semantic_text
                .as_deref()
                .map(|t| (plan.scope_str.as_str(), t)),
            status.as_u16(),
            content_type_of(&resp_headers),
            bytes.to_vec(),
        )
        .await;
    }
    Ok(llm::build_bytes_response(
        status,
        resp_headers,
        bytes.to_vec(),
    ))
}

/// Same-protocol streaming: tee the SSE into the exact cache on success, else pass
/// through. The SSE content type is only set for a genuine event stream (a 200
/// JSON body on a streaming request is an error and keeps its JSON type).
async fn fast_path_stream(
    state: &Arc<AppState>,
    out_codec: &dyn ProtocolCodec,
    upstream: reqwest::Response,
    status: reqwest::StatusCode,
    plan: &CachePlan,
) -> Result<Response, GatewayError> {
    // Only tee a genuine event stream into the cache; a non-2xx / non-SSE body is
    // passed through unchanged so its real status and content type are preserved.
    let sse = status.is_success() && is_event_stream(upstream.headers());
    let resp_headers = out_codec.response_headers(upstream.headers(), sse);
    let Some(key) = plan.store_key.clone().filter(|_| sse) else {
        return Ok(llm::build_response(upstream, resp_headers).await);
    };
    let ct = content_type_of(&resp_headers);
    let max = state.config.general_settings.cache.max_stream_bytes;
    let inner = upstream.bytes_stream().map_err(std::io::Error::other);
    Ok(llm::build_stream_response(
        StatusCode::OK,
        resp_headers,
        tee_and_store(
            state.clone(),
            key,
            status.as_u16(),
            ct,
            max,
            Box::pin(inner),
        ),
    ))
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
    plan: &CachePlan,
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
    maybe_inject_breakpoints(state, out_wire, &mut ir_req);
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
        return stream_cross_protocol(state, in_codec, out_codec, &ctx, upstream, plan);
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
    // Don't persist a translated provider failure as a cacheable success: either a
    // Responses HTTP-200 status:"failed", or any IR that parsed to an error stop.
    let failed = (out_wire == WireFormat::OpenAiResponses && is_failed_responses(&bytes))
        || matches!(ir_resp.stop_reason, Some(StopReason::Other(_)));
    if !failed && (plan.store_key.is_some() || plan.semantic_text.is_some()) {
        let ct = content_type_of(&resp_headers);
        store_response(
            state,
            plan.store_key.clone(),
            plan.semantic_text
                .as_deref()
                .map(|t| (plan.scope_str.as_str(), t)),
            status.as_u16(),
            ct,
            out_bytes.clone(),
        )
        .await;
    }
    Ok(llm::build_bytes_response(status, resp_headers, out_bytes))
}

/// Auto-inject Anthropic cache breakpoints for clients that can't express them,
/// when routed to an Anthropic upstream and the operator opted in.
fn maybe_inject_breakpoints(
    state: &Arc<AppState>,
    out_wire: WireFormat,
    ir_req: &mut crate::sdk::codec::ir::ChatRequest,
) {
    if out_wire != WireFormat::AnthropicMessages {
        return;
    }
    let pc = &state.config.general_settings.prompt_caching;
    if pc.enabled && pc.auto_inject {
        crate::sdk::codec::cache_inject::auto_inject_anthropic_breakpoints(
            ir_req,
            pc.max_breakpoints as usize,
            pc.min_tokens,
            pc.chars_per_token,
        );
    }
}

/// Bridge an upstream SSE stream back to the inbound protocol, optionally teeing
/// it into the exact-match cache.
fn stream_cross_protocol(
    state: &Arc<AppState>,
    in_codec: &dyn ProtocolCodec,
    out_codec: &dyn ProtocolCodec,
    ctx: &RequestCtx,
    upstream: reqwest::Response,
    plan: &CachePlan,
) -> Result<Response, GatewayError> {
    let cache_settings = &state.config.general_settings.cache;
    let resp_headers = in_codec.response_headers(upstream.headers(), true);
    let parser = out_codec.stream_parser();
    let renderer = in_codec.stream_renderer(ctx);
    let body_stream = transform_stream(upstream, parser, renderer);
    if let Some(key) = plan.store_key.clone() {
        let ct = content_type_of(&resp_headers);
        // Cross-protocol streaming always responds 200 (the live path does
        // too, above), so the cached status matches what a fresh call returns.
        return Ok(llm::build_stream_response(
            StatusCode::OK,
            resp_headers,
            tee_and_store(
                state.clone(),
                key,
                StatusCode::OK.as_u16(),
                ct,
                cache_settings.max_stream_bytes,
                Box::pin(body_stream),
            ),
        ));
    }
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
