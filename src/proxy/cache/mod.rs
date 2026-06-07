//! Exact-match response cache. On a hit the stored response is returned without
//! calling the upstream at all (0 tokens, 0 cost). Disabled by default; when off,
//! the request path is byte-identical to having no cache.
//!
//! Backends are dispatched through the [`ResponseCache`] enum rather than a
//! `dyn` trait to sidestep `async fn in trait` object-safety while keeping the
//! call sites boring. The in-memory backend is always available; Redis is behind
//! the `redis-cache` cargo feature.

pub mod key;
mod memory;
mod redb;
#[cfg(feature = "redis-cache")]
mod redis;
pub mod semantic;

use serde::{Deserialize, Serialize};

use crate::{
    errors::GatewayError,
    proxy::config::{CacheBackendKind, CacheSettings},
};

/// A cached upstream response, serialized so it can live in Redis as well as
/// in-memory. For streaming responses `body` is the full concatenated SSE bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub is_stream: bool,
}

pub enum ResponseCache {
    Disabled,
    Memory(memory::MemoryCache),
    Redb(redb::RedbCache),
    #[cfg(feature = "redis-cache")]
    Redis(Box<redis::RedisCache>),
}

impl std::fmt::Debug for ResponseCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Disabled => "Disabled",
            Self::Memory(_) => "Memory",
            Self::Redb(_) => "Redb",
            #[cfg(feature = "redis-cache")]
            Self::Redis(_) => "Redis",
        };
        write!(f, "ResponseCache::{name}")
    }
}

impl ResponseCache {
    /// Build the cache from config. Errors only on misconfiguration (e.g. the
    /// Redis backend selected without a URL or without the `redis-cache` feature).
    pub fn from_config(settings: &CacheSettings) -> Result<Self, GatewayError> {
        if !settings.enabled {
            return Ok(Self::Disabled);
        }
        match settings.backend {
            CacheBackendKind::Memory => Ok(Self::Memory(memory::MemoryCache::new(settings))),
            CacheBackendKind::Redb => Ok(Self::Redb(redb::RedbCache::from_config(settings)?)),
            CacheBackendKind::Redis => {
                #[cfg(feature = "redis-cache")]
                {
                    let url = settings.redis_url.clone().ok_or_else(|| {
                        GatewayError::InvalidConfig(
                            "cache.redis_url is required when cache.backend = redis".to_owned(),
                        )
                    })?;
                    Ok(Self::Redis(Box::new(redis::RedisCache::new(
                        url,
                        settings.ttl_secs,
                    ))))
                }
                #[cfg(not(feature = "redis-cache"))]
                {
                    Err(GatewayError::InvalidConfig(
                        "cache.backend = redis requires building with --features redis-cache"
                            .to_owned(),
                    ))
                }
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        match self {
            Self::Disabled => None,
            Self::Memory(m) => m.get(key).await,
            Self::Redb(c) => c.get(key).await,
            #[cfg(feature = "redis-cache")]
            Self::Redis(r) => r.get(key).await,
        }
    }

    pub async fn set(&self, key: String, value: CachedResponse) {
        match self {
            Self::Disabled => {}
            Self::Memory(m) => m.set(key, value).await,
            Self::Redb(c) => c.set(key, value).await,
            #[cfg(feature = "redis-cache")]
            Self::Redis(r) => r.set(key, value).await,
        }
    }
}
