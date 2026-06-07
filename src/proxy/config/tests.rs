use crate::proxy::config::cache::{CacheBackendKind, CacheSettings};
use crate::proxy::config::litellm_compat::{
    apply_cache_compat, litellm_settings_from_raw, LitellmSettingsCompat,
};

fn ls(yaml: &str) -> LitellmSettingsCompat {
    serde_yaml::from_str(yaml).unwrap()
}

#[test]
fn maps_upstream_redis_cache_params() {
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls(r#"
cache: true
cache_params:
  type: redis
  host: localhost
  port: 6379
  password: secret
  ttl: 600
"#),
    )
    .unwrap();
    assert!(cache.enabled);
    assert_eq!(cache.backend, CacheBackendKind::Redis);
    assert_eq!(
        cache.redis_url.as_deref(),
        Some("redis://:secret@localhost:6379")
    );
    assert_eq!(cache.ttl_secs, 600);
}

#[test]
fn maps_upstream_disk_to_redb() {
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls(r#"
cache: true
cache_params:
  type: disk
  disk_cache_dir: /var/cache/litellm
  ttl: 120
"#),
    )
    .unwrap();
    assert!(cache.enabled);
    assert_eq!(cache.backend, CacheBackendKind::Redb);
    assert_eq!(
        cache.redb_path.as_deref(),
        Some("/var/cache/litellm/litellm-cache.redb")
    );
    assert_eq!(cache.ttl_secs, 120);
}

#[test]
fn absent_type_defaults_to_redis() {
    // Upstream proxy defaults an unset cache type to redis.
    let mut cache = CacheSettings::default();
    apply_cache_compat(&mut cache, &ls("cache: true\ncache_params:\n  host: r\n")).unwrap();
    assert_eq!(cache.backend, CacheBackendKind::Redis);
    assert_eq!(cache.redis_url.as_deref(), Some("redis://r:6379"));
}

#[test]
fn native_cache_takes_precedence() {
    let mut cache = CacheSettings {
        enabled: true,
        backend: CacheBackendKind::Memory,
        ..Default::default()
    };
    apply_cache_compat(
        &mut cache,
        &ls("cache: true\ncache_params:\n  type: redis\n  host: x\n"),
    )
    .unwrap();
    assert_eq!(cache.backend, CacheBackendKind::Memory);
    assert!(cache.redis_url.is_none());
}

#[test]
fn unsupported_type_leaves_cache_off() {
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls("cache: true\ncache_params:\n  type: s3\n  s3_bucket_name: b\n"),
    )
    .unwrap();
    assert!(!cache.enabled);
}

#[test]
fn cache_not_enabled_is_ignored() {
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls("cache: false\ncache_params:\n  type: redis\n  host: x\n"),
    )
    .unwrap();
    assert!(!cache.enabled);
}

#[test]
fn from_raw_extracts_only_litellm_settings() {
    let raw = "model_list: []\nlitellm_settings:\n  cache: true\n  cache_params:\n    type: disk\n";
    let parsed = litellm_settings_from_raw(raw).unwrap().unwrap();
    assert_eq!(parsed.cache, Some(true));
    assert!(litellm_settings_from_raw("model_list: []\n")
        .unwrap()
        .is_none());
}

#[test]
fn malformed_litellm_settings_is_ignored_not_fatal() {
    // A litellm_settings block we can't shape (cache_params as a scalar) must
    // degrade to "ignored", never abort the whole config load.
    let raw = "model_list: []\nlitellm_settings:\n  cache: true\n  cache_params: not-a-map\n";
    assert!(litellm_settings_from_raw(raw).unwrap().is_none());
}

#[test]
fn tolerates_quoted_ttl() {
    // A quoted ttl ("600") would fail a strict f64 field; it must still apply.
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls("cache: true\ncache_params:\n  type: local\n  ttl: \"600\"\n"),
    )
    .unwrap();
    assert_eq!(cache.backend, CacheBackendKind::Memory);
    assert_eq!(cache.ttl_secs, 600);
}

#[test]
fn synthesizes_rediss_url_with_username_and_encoded_password() {
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls(r#"
cache: true
cache_params:
  type: redis
  host: redis.example.com
  port: 6380
  username: admin
  password: "p@ss/w:rd"
  ssl: true
"#),
    )
    .unwrap();
    assert_eq!(
        cache.redis_url.as_deref(),
        Some("rediss://admin:p%40ss%2Fw%3Ard@redis.example.com:6380")
    );
}

#[test]
fn redis_url_env_fallback_when_no_host() {
    std::env::set_var("REDIS_URL", "redis://from-env:6379");
    let mut cache = CacheSettings::default();
    apply_cache_compat(
        &mut cache,
        &ls("cache: true\ncache_params:\n  type: redis\n"),
    )
    .unwrap();
    assert_eq!(cache.redis_url.as_deref(), Some("redis://from-env:6379"));
    std::env::remove_var("REDIS_URL");
}
