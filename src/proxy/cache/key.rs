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

/// Whether a request is safe to cache. Requests with `temperature > 0` are
/// non-deterministic and skipped unless the operator opted in. An absent
/// temperature is treated as cacheable (an identical request is served the same
/// stored answer, which is the point of an opt-in cache).
pub fn is_deterministic(body: &Value, settings: &CacheSettings) -> bool {
    if settings.cache_non_deterministic {
        return true;
    }
    match body.get("temperature").and_then(Value::as_f64) {
        Some(t) => t == 0.0,
        None => true,
    }
}

/// Hash a caller credential for tenant scoping. Never stores the raw key.
pub fn hash_scope(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

/// Build the cache key for a request routed to a specific deployment.
pub fn build_key(
    scope: Option<&str>,
    inbound_wire: WireFormat,
    provider_id: &str,
    api_base: &str,
    upstream_model: &str,
    stream: bool,
    body: &Value,
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
    // Canonical because serde_json (no preserve_order) sorts object keys.
    hasher.update(&serde_json::to_vec(body).unwrap_or_default());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(body: &Value) -> String {
        build_key(
            Some("tenant"),
            WireFormat::AnthropicMessages,
            "anthropic",
            "https://api",
            "claude",
            false,
            body,
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
        let a = build_key(
            Some("t1"),
            WireFormat::AnthropicMessages,
            "anthropic",
            "https://api",
            "claude",
            false,
            &body,
        );
        let b = build_key(
            Some("t2"),
            WireFormat::AnthropicMessages,
            "anthropic",
            "https://api",
            "claude",
            false,
            &body,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn stream_flag_changes_key() {
        let body = json!({"m": 1});
        let a = build_key(None, WireFormat::OpenAiChat, "p", "b", "m", false, &body);
        let b = build_key(None, WireFormat::OpenAiChat, "p", "b", "m", true, &body);
        assert_ne!(a, b);
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
        assert!(is_deterministic(&json!({}), &s));
        assert!(is_deterministic(&json!({"temperature": 0.0}), &s));
        assert!(!is_deterministic(&json!({"temperature": 0.7}), &s));
        let s2 = CacheSettings {
            cache_non_deterministic: true,
            ..Default::default()
        };
        assert!(is_deterministic(&json!({"temperature": 0.7}), &s2));
    }
}
