//! In-process response cache backed by `moka` (async, TTL + size eviction).

use std::time::Duration;

use moka::future::Cache;

use super::CachedResponse;
use crate::proxy::config::CacheSettings;

pub struct MemoryCache {
    inner: Cache<String, CachedResponse>,
}

impl MemoryCache {
    pub fn new(settings: &CacheSettings) -> Self {
        let inner = Cache::builder()
            .max_capacity(settings.max_entries)
            .time_to_live(Duration::from_secs(settings.ttl_secs))
            .build();
        Self { inner }
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        self.inner.get(key).await
    }

    pub async fn set(&self, key: String, value: CachedResponse) {
        self.inner.insert(key, value).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(ttl: u64, max: u64) -> CacheSettings {
        CacheSettings {
            enabled: true,
            ttl_secs: ttl,
            max_entries: max,
            ..Default::default()
        }
    }

    fn resp(body: &str) -> CachedResponse {
        CachedResponse {
            status: 200,
            content_type: "application/json".to_owned(),
            body: body.as_bytes().to_vec(),
            is_stream: false,
        }
    }

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let c = MemoryCache::new(&settings(300, 10));
        assert!(c.get("k").await.is_none());
        c.set("k".to_owned(), resp("hello")).await;
        assert_eq!(c.get("k").await.unwrap().body, b"hello");
    }

    #[tokio::test]
    async fn entries_expire_after_ttl() {
        let c = MemoryCache::new(&settings(1, 10));
        c.set("k".to_owned(), resp("x")).await;
        // moka uses a quiescent clock; run_pending_tasks + a sleep past the TTL.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        c.inner.run_pending_tasks().await;
        assert!(c.get("k").await.is_none());
    }
}
