//! Cache key construction and per-request cache directives.
//!
//! The key hashes the deployment scope (tenant + provider + model + wire +
//! stream flag) together with the canonical request body. Because `serde_json`
//! is built without `preserve_order`, a `Value` re-serializes with sorted object
//! keys, so two logically identical requests hash the same regardless of the
//! client's original key ordering. Auth headers and request ids are never part of
//! the key (they live in headers, not the body).

use axum::http::{header::CACHE_CONTROL, HeaderMap};
use serde_json::Value;

use crate::{proxy::config::CacheSettings, sdk::codec::WireFormat};

/// Per-request cache behaviour parsed from request headers and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheDirective {
    /// Attempt a cache read (return a stored response on hit).
    pub read: bool,
    /// Store the upstream response on a miss.
    pub store: bool,
}

/// Parse per-request cache opt-outs. `no-cache` bypasses the read (and the
/// store); `no-store` keeps reads but won't persist new responses. Read from HTTP
/// `Cache-Control` / `x-no-cache` headers and, for parity with upstream litellm,
/// from the request body's `cache: { "no-cache": .., "no-store": .. }` param.
pub fn read_directive(headers: &HeaderMap, body: &Value) -> CacheDirective {
    let cc = headers
        .get(CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let x_no_cache = headers
        .get("x-no-cache")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");
    // Upstream litellm reads `cache: {...}` from the request body (hyphenated keys).
    let body_cache = body.get("cache");
    let body_flag = |name: &str| {
        body_cache
            .and_then(|c| c.get(name))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let no_cache = cc.contains("no-cache") || x_no_cache || body_flag("no-cache");
    let no_store = cc.contains("no-store") || body_flag("no-store");
    CacheDirective {
        read: !no_cache,
        store: !no_cache && !no_store,
    }
}

/// Whether a request is safe to cache. Only an explicit `temperature == 0` is
/// deterministic. A request that omits `temperature` relies on the provider's
/// (nonzero) sampling default, so it is non-deterministic too — caching it would
/// replay one stochastic answer for every later identical prompt. Operators who
/// want to cache regardless set `cache_non_deterministic`.
pub fn is_deterministic(body: &Value, settings: &CacheSettings) -> bool {
    if settings.cache_non_deterministic {
        return true;
    }
    // OpenAI/Anthropic put `temperature` at the top level; native Gemini nests it
    // under `generationConfig` (camelCase REST) / `generation_config`.
    let temperature = body
        .get("temperature")
        .or_else(|| nested_temperature(body, "generationConfig"))
        .or_else(|| nested_temperature(body, "generation_config"))
        .and_then(Value::as_f64);
    temperature == Some(0.0)
}

fn nested_temperature<'a>(body: &'a Value, key: &str) -> Option<&'a Value> {
    body.get(key).and_then(|g| g.get("temperature"))
}

/// Hash a caller credential for tenant scoping. Never stores the raw key.
pub fn hash_scope(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

/// Build the cache key for a request routed to a specific deployment. `key_headers`
/// names the inbound headers that shape the upstream response (supplied by the
/// outbound codec via `cache_key_headers`), so the key stays aligned with whatever
/// that codec actually forwards.
#[allow(clippy::too_many_arguments)]
pub fn build_key(
    scope: Option<&str>,
    inbound_wire: WireFormat,
    provider_id: &str,
    api_base: &str,
    upstream_model: &str,
    stream: bool,
    body: &Value,
    headers: &HeaderMap,
    key_headers: &[&str],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scope.unwrap_or("").as_bytes());
    hasher.update(&[0]);
    hasher.update(&[inbound_wire as u8]);
    hasher.update(provider_id.as_bytes());
    hasher.update(&[0]);
    hasher.update(api_base.as_bytes());
    hasher.update(&[0]);
    hasher.update(upstream_model.as_bytes());
    hasher.update(&[stream as u8]);
    // Response-shaping headers, hashed in the codec's declared order for stability.
    for name in key_headers {
        if let Some(value) = headers.get(*name) {
            hasher.update(name.as_bytes());
            hasher.update(&[0]);
            hasher.update(value.as_bytes());
            hasher.update(&[0]);
        }
    }
    // Canonical because serde_json (no preserve_order) sorts object keys.
    hasher.update(&serde_json::to_vec(body).unwrap_or_default());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bk(
        scope: Option<&str>,
        wire: WireFormat,
        stream: bool,
        body: &Value,
        h: &HeaderMap,
    ) -> String {
        build_key(
            scope,
            wire,
            "anthropic",
            "https://api",
            "claude",
            stream,
            body,
            h,
            &["anthropic-beta", "anthropic-version"],
        )
    }

    fn key(body: &Value) -> String {
        bk(
            Some("tenant"),
            WireFormat::AnthropicMessages,
            false,
            body,
            &HeaderMap::new(),
        )
    }

    #[test]
    fn key_is_stable_under_object_key_reordering() {
        let a = json!({"model": "x", "messages": [{"role": "user", "content": "hi"}]});
        let b = json!({"messages": [{"content": "hi", "role": "user"}], "model": "x"});
        assert_eq!(key(&a), key(&b));
    }

    #[test]
    fn different_body_yields_different_key() {
        assert_ne!(key(&json!({"m": 1})), key(&json!({"m": 2})));
    }

    #[test]
    fn tenant_scope_changes_key() {
        let body = json!({"m": 1});
        let none = HeaderMap::new();
        let a = bk(
            Some("t1"),
            WireFormat::AnthropicMessages,
            false,
            &body,
            &none,
        );
        let b = bk(
            Some("t2"),
            WireFormat::AnthropicMessages,
            false,
            &body,
            &none,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn stream_flag_changes_key() {
        let body = json!({"m": 1});
        let h = HeaderMap::new();
        let a = bk(None, WireFormat::OpenAiChat, false, &body, &h);
        let b = bk(None, WireFormat::OpenAiChat, true, &body, &h);
        assert_ne!(a, b);
    }

    #[test]
    fn response_shaping_header_changes_key() {
        let body = json!({"m": 1});
        let wire = WireFormat::AnthropicMessages;
        let base = bk(None, wire, false, &body, &HeaderMap::new());
        let mut h = HeaderMap::new();
        h.insert(
            "anthropic-beta",
            "prompt-caching-2024-07-31".parse().unwrap(),
        );
        assert_ne!(base, bk(None, wire, false, &body, &h));
        // A volatile per-request header is ignored, so the cache still hits.
        let mut h2 = HeaderMap::new();
        h2.insert("session-id", "abc-123".parse().unwrap());
        assert_eq!(base, bk(None, wire, false, &body, &h2));
    }

    #[test]
    fn directive_parses_no_cache_and_no_store() {
        let body = json!({});
        let mut h = HeaderMap::new();
        assert_eq!(
            read_directive(&h, &body),
            CacheDirective {
                read: true,
                store: true
            }
        );
        h.insert(CACHE_CONTROL, "no-store".parse().unwrap());
        assert_eq!(
            read_directive(&h, &body),
            CacheDirective {
                read: true,
                store: false
            }
        );
        h.insert(CACHE_CONTROL, "no-cache".parse().unwrap());
        assert_eq!(
            read_directive(&h, &body),
            CacheDirective {
                read: false,
                store: false
            }
        );
    }

    #[test]
    fn directive_honours_body_cache_param() {
        let h = HeaderMap::new();
        // Upstream litellm body shape: `cache: { "no-cache": true }`.
        let no_cache = json!({"cache": {"no-cache": true}});
        assert_eq!(
            read_directive(&h, &no_cache),
            CacheDirective {
                read: false,
                store: false
            }
        );
        let no_store = json!({"cache": {"no-store": true}});
        assert_eq!(
            read_directive(&h, &no_store),
            CacheDirective {
                read: true,
                store: false
            }
        );
        // No `cache` param leaves the default behaviour intact.
        assert_eq!(
            read_directive(&h, &json!({"model": "x"})),
            CacheDirective {
                read: true,
                store: true
            }
        );
    }

    #[test]
    fn determinism_respects_temperature() {
        let s = CacheSettings::default();
        // Omitted temperature relies on the provider's nonzero default → not cacheable.
        assert!(!is_deterministic(&json!({}), &s));
        assert!(is_deterministic(&json!({"temperature": 0.0}), &s));
        assert!(!is_deterministic(&json!({"temperature": 0.7}), &s));
        // Native Gemini nests temperature under generationConfig.
        assert!(is_deterministic(
            &json!({"generationConfig": {"temperature": 0.0}}),
            &s
        ));
        let s2 = CacheSettings {
            cache_non_deterministic: true,
            ..Default::default()
        };
        assert!(is_deterministic(&json!({"temperature": 0.7}), &s2));
    }
}
