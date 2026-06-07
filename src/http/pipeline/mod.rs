//! Request pipeline: translate inbound → IR → outbound, send upstream, then
//! translate the response back to the inbound protocol. When inbound and
//! outbound wire formats match, the body is passed through untouched (fast
//! path), preserving the original low-overhead behaviour.

mod cache;
mod dispatch;
mod transform;

use std::sync::Arc;

use axum::{http::HeaderMap, response::Response};
use serde_json::Value;

use crate::{
    errors::GatewayError,
    http::credential_overrides,
    proxy::{
        auth::master_key::presented_key,
        cache::{key as cache_key, semantic},
        state::AppState,
    },
    sdk::codec::{codec_for, WireFormat},
};

use cache::replay_cached;

/// Cache decisions resolved up front: what (if anything) to store after the
/// upstream call, plus the per-tenant scope string for semantic recording.
struct CachePlan {
    /// Exact-match cache key to write on a successful miss, if any.
    pub(super) store_key: Option<String>,
    /// Semantic query text to record on a successful miss, if any.
    pub(super) semantic_text: Option<String>,
    /// Semantic recording namespace: tenant scope qualified by deployment
    /// identity (provider/base/model) so entries can't cross models.
    pub(super) scope_str: String,
}

/// Outcome of cache planning: either an early cache-hit response, or the plan
/// describing what to store after the live call.
enum CacheOutcome {
    Hit(Response),
    Plan(CachePlan),
}

/// Drive one request through the gateway. `model` is the public model name (from
/// the body or, for Gemini, the URL path); `stream` is the resolved streaming
/// flag for the request.
pub async fn handle(
    state: &Arc<AppState>,
    inbound_wire: WireFormat,
    model: String,
    stream: bool,
    mut body: Value,
    inbound_headers: &HeaderMap,
) -> Result<Response, GatewayError> {
    let route = credential_overrides::apply(state, state.router.resolve(&model)?).await?;
    let deployment = route.deployment;
    let out_wire = deployment.wire;
    let url = deployment.upstream_url(stream);

    let plan = match plan_cache(
        state,
        inbound_wire,
        &deployment,
        stream,
        &mut body,
        inbound_headers,
    )
    .await
    {
        CacheOutcome::Hit(resp) => return Ok(resp),
        CacheOutcome::Plan(plan) => plan,
    };

    if inbound_wire == out_wire {
        dispatch::run_fast_path(
            state,
            &deployment,
            url,
            stream,
            body,
            inbound_headers,
            &plan,
        )
        .await
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
            &plan,
        )
        .await
    }
}

/// Read both caches (exact-match + semantic) and decide what to store later.
/// Returns an early `Hit` response when either cache serves the request. Strips
/// the proprietary `cache` control field from `body` as a side effect.
async fn plan_cache(
    state: &Arc<AppState>,
    inbound_wire: WireFormat,
    deployment: &crate::sdk::router::Deployment,
    stream: bool,
    body: &mut Value,
    inbound_headers: &HeaderMap,
) -> CacheOutcome {
    // Response cache (exact-match) + optional semantic cache. Both try a read and
    // remember what to store on a miss. Skipped entirely when disabled, leaving
    // the request path unchanged.
    let cache_settings = &state.config.general_settings.cache;
    let any_cache = state.cache.is_enabled() || state.semantic.is_enabled();
    let directive = resolve_directive(any_cache, inbound_headers, body);
    let scope = if any_cache && cache_settings.scope_by_api_key {
        presented_key(inbound_headers).map(cache_key::hash_scope)
    } else {
        None
    };
    let scope_str = scope.as_deref().unwrap_or("").to_owned();
    // With per-tenant scoping on, a request that presents no API key cannot be
    // safely isolated — caching it would let unauthenticated callers share (and
    // leak) each other's responses. So such requests neither read nor write.
    let scope_ok = !cache_settings.scope_by_api_key || scope.is_some();

    let store_key = match try_exact_cache(
        state,
        inbound_wire,
        deployment,
        stream,
        body,
        &directive,
        scope,
        scope_ok,
        inbound_headers,
    )
    .await
    {
        Err(hit) => return CacheOutcome::Hit(hit),
        Ok(key) => key,
    };

    let semantic_scope = semantic_scope(
        &scope_str,
        inbound_wire,
        deployment,
        inbound_headers,
        codec_for(deployment.wire).cache_key_headers(),
    );

    let semantic_text = match try_semantic_cache(
        state,
        stream,
        body,
        &directive,
        &semantic_scope,
        scope_ok,
    )
    .await
    {
        Err(hit) => return CacheOutcome::Hit(hit),
        Ok(text) => text,
    };

    CacheOutcome::Plan(CachePlan {
        store_key,
        semantic_text,
        scope_str: semantic_scope,
    })
}

/// Resolve the per-request cache directive and always strip the gateway-only
/// `cache` control field (read first when enabled) so it never reaches upstream.
fn resolve_directive(
    any_cache: bool,
    headers: &HeaderMap,
    body: &mut Value,
) -> cache_key::CacheDirective {
    let directive = if any_cache {
        cache_key::read_directive(headers, body)
    } else {
        cache_key::CacheDirective {
            read: false,
            store: false,
        }
    };
    if let Some(obj) = body.as_object_mut() {
        obj.remove("cache");
    }
    directive
}

/// Semantic recording namespace. Mirrors the exact cache key's isolation (tenant,
/// inbound + outbound wire, provider/base/model, response-shaping headers) so a
/// stored client body can't be replayed across a dimension that changes its shape.
fn semantic_scope(
    scope_str: &str,
    inbound_wire: WireFormat,
    deployment: &crate::sdk::router::Deployment,
    headers: &HeaderMap,
    key_headers: &[&str],
) -> String {
    let mut scope = format!(
        "{scope_str}\0{}\0{}\0{}\0{}\0{}",
        inbound_wire as u8,
        deployment.wire as u8,
        deployment.provider_id,
        deployment.api_base,
        deployment.upstream_model
    );
    for name in key_headers {
        if let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            scope.push('\0');
            scope.push_str(name);
            scope.push('\0');
            scope.push_str(value);
        }
    }
    scope
}

/// Exact-match cache lookup. `Err(response)` on a hit; `Ok(Some(key))` is the key
/// to store on a later miss, `Ok(None)` when this request is not exact-cacheable.
#[allow(clippy::too_many_arguments)]
async fn try_exact_cache(
    state: &Arc<AppState>,
    inbound_wire: WireFormat,
    deployment: &crate::sdk::router::Deployment,
    stream: bool,
    body: &Value,
    directive: &cache_key::CacheDirective,
    scope: Option<String>,
    scope_ok: bool,
    inbound_headers: &HeaderMap,
) -> Result<Option<String>, Response> {
    let cache_settings = &state.config.general_settings.cache;
    if !(state.cache.is_enabled()
        && scope_ok
        && (directive.read || directive.store)
        && (!stream || cache_settings.cache_streaming)
        && cache_key::is_deterministic(body, cache_settings))
    {
        return Ok(None);
    }
    let key = cache_key::build_key(
        scope.as_deref(),
        inbound_wire,
        deployment.wire,
        &deployment.provider_id,
        &deployment.api_base,
        &deployment.upstream_model,
        stream,
        body,
        inbound_headers,
        codec_for(deployment.wire).cache_key_headers(),
    );
    if directive.read {
        if let Some(hit) = state.cache.get(&key).await {
            return Err(replay_cached(hit, "hit"));
        }
    }
    Ok(directive.store.then_some(key))
}

/// Semantic cache lookup (deterministic, tool-free, non-streaming only).
/// `Err(response)` on a hit; `Ok(Some(text))` is the query text to record on a
/// later miss, `Ok(None)` when this request is not semantically cacheable.
async fn try_semantic_cache(
    state: &Arc<AppState>,
    stream: bool,
    body: &Value,
    directive: &cache_key::CacheDirective,
    scope_str: &str,
    scope_ok: bool,
) -> Result<Option<String>, Response> {
    let cache_settings = &state.config.general_settings.cache;
    if !(state.semantic.is_enabled()
        && scope_ok
        && !stream
        && (directive.read || directive.store)
        && cache_key::is_deterministic(body, cache_settings)
        && semantic::eligible(body, &cache_settings.semantic))
    {
        return Ok(None);
    }
    let text = semantic::query_text(body);
    if directive.read {
        if let Some(hit) = state.semantic.lookup(scope_str, &text).await {
            return Err(replay_cached(hit, "semantic"));
        }
    }
    Ok(directive.store.then_some(text))
}
