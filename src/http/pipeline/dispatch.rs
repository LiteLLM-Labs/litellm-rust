//! Same-protocol fast path and cross-protocol translation dispatch.

use std::sync::Arc;

use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::TryStreamExt;
use serde_json::{json, Value};

use crate::{
    errors::GatewayError,
    http::llm,
    proxy::state::AppState,
    sdk::codec::{codec_for, ir::StopReason, ProtocolCodec, RequestCtx, WireFormat},
};

use super::cache::{content_type_of, store_response, tee_and_store};
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
    let cache_settings = &state.config.general_settings.cache;
    let out_codec = codec_for(deployment.wire);
    rewrite_model(&mut body, deployment);
    let headers = out_codec.outbound_headers(deployment, inbound_headers)?;
    let upstream = llm::send_request(&state.http, url, serde_json::to_vec(&body)?, headers).await?;
    let status = upstream.status();
    // Only successful streams are SSE; an error body keeps its JSON content type.
    let resp_headers =
        out_codec.response_headers(upstream.headers(), stream && status.is_success());
    // Only cache successful responses; errors pass through unstored.
    let want_store =
        status.is_success() && (plan.store_key.is_some() || plan.semantic_text.is_some());
    if !want_store {
        return Ok(llm::build_response(upstream, resp_headers).await);
    }
    let ct = content_type_of(&resp_headers);
    if stream {
        // Semantic caching is non-streaming; only the exact key applies.
        if let Some(key) = plan.store_key.clone() {
            let inner = upstream.bytes_stream().map_err(std::io::Error::other);
            return Ok(llm::build_stream_response(
                StatusCode::OK,
                resp_headers,
                tee_and_store(
                    state.clone(),
                    key,
                    status.as_u16(),
                    ct,
                    cache_settings.max_stream_bytes,
                    Box::pin(inner),
                ),
            ));
        }
        return Ok(llm::build_response(upstream, resp_headers).await);
    }
    let bytes = upstream.bytes().await.map_err(GatewayError::Upstream)?;
    // Providers can report failure in an HTTP-200 body: Responses `status:"failed"`
    // or an OpenAI/Anthropic top-level `error` object. Neither is cacheable.
    let failed = is_failed_responses(&bytes) || has_error_object(&bytes);
    if !failed {
        store_response(
            state,
            plan.store_key.clone(),
            plan.semantic_text
                .as_deref()
                .map(|t| (plan.scope_str.as_str(), t)),
            status.as_u16(),
            ct,
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

/// Rewrite the body's `model` to the upstream name on the same-protocol path.
/// Gemini carries the model in the URL, so its body has none to rewrite.
fn rewrite_model(body: &mut Value, deployment: &crate::sdk::router::Deployment) {
    if deployment.wire != WireFormat::Gemini
        && body.get("model").and_then(Value::as_str) != Some(deployment.upstream_model.as_str())
    {
        body["model"] = json!(deployment.upstream_model);
    }
}

/// True when a Responses JSON body reports `status: "failed"` despite HTTP 2xx.
fn is_failed_responses(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        == Some("failed")
}

/// True when a 2xx JSON body carries a top-level `error` object (OpenAI/Anthropic
/// stream a failure this way without a non-2xx status).
fn has_error_object(bytes: &[u8]) -> bool {
    // Require a non-null `error`: successful Responses objects carry `error: null`.
    matches!(
        serde_json::from_slice::<Value>(bytes),
        Ok(Value::Object(ref o)) if o.get("error").is_some_and(|e| !e.is_null())
    )
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
    if !status.is_success() {
        // Provider errors are passed through as-is, not translated.
        let err_headers = in_codec.response_headers(upstream.headers(), false);
        return Ok(llm::build_response(upstream, err_headers).await);
    }

    if stream {
        return stream_cross_protocol(state, in_codec, out_codec, &ctx, upstream, plan);
    }

    let resp_headers = in_codec.response_headers(upstream.headers(), false);
    let bytes = upstream.bytes().await.map_err(GatewayError::Upstream)?;
    // A 200 body carrying a top-level error (OpenAI Chat / Anthropic) parses to an
    // empty IR; pass it through untranslated and uncached rather than emit a
    // success-looking empty response and poison the cache. Responses failures are
    // excluded — their codec translates `status:"failed"` into a proper IR error.
    if out_wire != WireFormat::OpenAiResponses && has_error_object(&bytes) {
        return Ok(llm::build_bytes_response(
            status,
            resp_headers,
            bytes.to_vec(),
        ));
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
