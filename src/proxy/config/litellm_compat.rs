use std::{collections::HashMap, path::Path};

use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;

use crate::errors::GatewayError;
use crate::proxy::config::cache::{CacheBackendKind, CacheSettings};
use crate::proxy::config::load::expand_env_value;

/// Subset of an upstream litellm `litellm_settings` block we know how to honour.
/// Only the response-cache stanza is translated; every other key is ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct LitellmSettingsCompat {
    /// Upstream master switch (`litellm_settings.cache: true`).
    #[serde(default)]
    pub cache: Option<bool>,
    /// Upstream `litellm_settings.cache_params`.
    #[serde(default)]
    pub cache_params: Option<LitellmCacheParams>,
}

/// The upstream `cache_params` keys litellm-rust can map onto its native cache.
/// Only `type`/`disk_cache_dir` are typed; `ttl`, `host`/`port`/`password`/
/// `username`/`ssl` and any unsupported keys stay in `extra` and are read
/// leniently (string or number) so an odd-but-valid value never fails the load.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LitellmCacheParams {
    #[serde(rename = "type")]
    pub cache_type: Option<String>,
    pub disk_cache_dir: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

/// Pull an upstream `litellm_settings` block straight out of the raw YAML. It is
/// not a field on [`GatewayConfig`] (litellm-rust configures everything under
/// `general_settings`); this read-only shim only exists to translate the cache
/// stanza, so we parse it on the side rather than widen the typed config.
pub(crate) fn litellm_settings_from_raw(
    raw: &str,
) -> Result<Option<LitellmSettingsCompat>, GatewayError> {
    let doc: serde_yaml::Value = serde_yaml::from_str(raw)?;
    let Some(node) = doc.get("litellm_settings") else {
        return Ok(None);
    };
    // Best-effort: a malformed litellm_settings block must not abort startup for an
    // otherwise-valid config (litellm-rust configures caching under general_settings).
    match serde_yaml::from_value::<LitellmSettingsCompat>(node.clone()) {
        Ok(settings) => Ok(Some(settings)),
        Err(e) => {
            tracing::warn!(
                "litellm_settings could not be read ({e}); ignoring it — configure \
                 response caching under general_settings.cache"
            );
            Ok(None)
        }
    }
}

/// Translate an upstream litellm `litellm_settings.cache` block into the native
/// `general_settings.cache` so an existing litellm config.yaml keeps caching after
/// a drop-in migration. Native `general_settings.cache` wins when caching is
/// already enabled there; otherwise an upstream `cache: true` block is honoured.
/// Runs after `expand_env`, so it expands `os.environ/…` in the values it reads.
/// Unsupported `type`s / keys are warned about, never silently dropped.
pub(crate) fn apply_cache_compat(
    cache: &mut CacheSettings,
    ls: &LitellmSettingsCompat,
) -> Result<(), GatewayError> {
    // The native config takes precedence once the operator explicitly enabled it.
    if cache.enabled {
        return Ok(());
    }
    if ls.cache != Some(true) {
        return Ok(());
    }
    let params = ls.cache_params.clone().unwrap_or_default();

    // Upstream defaults an absent `type` to "redis" (matching the upstream proxy).
    let backend = match params
        .cache_type
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("local") => CacheBackendKind::Memory,
        Some("disk") => CacheBackendKind::Redb,
        Some("redis") | None => CacheBackendKind::Redis,
        Some(other) => {
            tracing::warn!(
                "litellm_settings.cache_params.type = {other} is not supported by litellm-rust \
                 (supported: local→memory, disk→redb, redis); response caching stays off — \
                 configure general_settings.cache instead"
            );
            return Ok(());
        }
    };
    warn_unsupported_cache_params(&params);

    let synthesized_redis_url = if backend == CacheBackendKind::Redis {
        synth_redis_url(&params)?
    } else {
        None
    };
    cache.enabled = true;
    cache.backend = backend;
    // `ttl` is read leniently (string or number, env-expandable) and applied only
    // at whole-second granularity; a sub-second value would truncate to 0 (a
    // useless permanent miss), so it falls back to the default instead.
    let ttl = params
        .extra
        .get("ttl")
        .and_then(yaml_scalar)
        .map(|s| expand_env_value(&s))
        .transpose()?
        .and_then(|s| s.parse::<f64>().ok());
    if let Some(ttl) = ttl {
        if ttl >= 1.0 {
            cache.ttl_secs = ttl as u64;
        }
    }
    match backend {
        CacheBackendKind::Redis if cache.redis_url.is_none() => {
            cache.redis_url = synthesized_redis_url
        }
        CacheBackendKind::Redb if cache.redb_path.is_none() => {
            if let Some(dir) = params.disk_cache_dir.as_deref() {
                // Upstream `disk_cache_dir` names a directory; redb is a single
                // file, so place the db inside it rather than colliding with it.
                let path = Path::new(&expand_env_value(dir)?).join("litellm-cache.redb");
                cache.redb_path = Some(path.to_string_lossy().into_owned());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Coerce a YAML scalar (string/number/bool) to a string; `None` for collections.
fn yaml_scalar(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// RFC 3986 userinfo percent-encode set (matches the `url` crate), so a Redis
/// `username`/`password` with reserved chars (`/`, `:`, `@`, …) yields a URL the
/// redis client can parse instead of silently failing to connect.
const USERINFO: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'|')
    .add(b'%');

/// Build `redis(s)://[user]:[password]@host:port` from upstream
/// `host`/`port`/`username`/`password`/`ssl`. With no host, fall back to a
/// `REDIS_URL` env var (the common Docker/k8s pattern upstream also supports);
/// other `REDIS_*` vars are out of scope. `None` only when nothing is configured.
fn synth_redis_url(params: &LitellmCacheParams) -> Result<Option<String>, GatewayError> {
    let Some(host) = params.extra.get("host").and_then(yaml_scalar) else {
        return Ok(std::env::var("REDIS_URL").ok());
    };
    let host = expand_env_value(&host)?;
    let port = match params.extra.get("port").and_then(yaml_scalar) {
        Some(p) => expand_env_value(&p)?,
        None => "6379".to_owned(),
    };
    let scheme = if params.extra.get("ssl").is_some_and(yaml_truthy) {
        "rediss"
    } else {
        "redis"
    };
    let userinfo = redis_userinfo(params)?;
    Ok(Some(format!("{scheme}://{userinfo}{host}:{port}")))
}

/// `user:password@` with each segment percent-encoded, or empty when neither set.
fn redis_userinfo(params: &LitellmCacheParams) -> Result<String, GatewayError> {
    let user = match params.extra.get("username").and_then(yaml_scalar) {
        Some(u) => expand_env_value(&u)?,
        None => String::new(),
    };
    let pass = match params.extra.get("password").and_then(yaml_scalar) {
        Some(p) => expand_env_value(&p)?,
        None => String::new(),
    };
    if user.is_empty() && pass.is_empty() {
        return Ok(String::new());
    }
    let enc = |s: &str| percent_encode(s.as_bytes(), USERINFO).to_string();
    Ok(format!("{}:{}@", enc(&user), enc(&pass)))
}

/// Interpret a YAML scalar as a boolean flag (`true`/`1`/`yes`, or non-zero).
fn yaml_truthy(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::Bool(b) => *b,
        serde_yaml::Value::String(s) => {
            matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
        }
        serde_yaml::Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => false,
    }
}

/// Warn (once) about upstream `cache_params` keys litellm-rust cannot honour, so
/// they fail loud in the log instead of vanishing silently.
fn warn_unsupported_cache_params(params: &LitellmCacheParams) {
    const UNSUPPORTED: &[&str] = &[
        "mode",
        "namespace",
        "default_in_memory_ttl",
        "default_in_redis_ttl",
        "supported_call_types",
        "similarity_threshold",
    ];
    let present: Vec<&str> = UNSUPPORTED
        .iter()
        .copied()
        .filter(|k| params.extra.contains_key(*k))
        .collect();
    if !present.is_empty() {
        tracing::warn!(
            "litellm_settings.cache_params keys ignored by litellm-rust: {}; \
             see docs/protocols.md for the supported general_settings.cache options",
            present.join(", ")
        );
    }
}
