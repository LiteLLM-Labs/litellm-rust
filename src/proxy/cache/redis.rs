//! Redis response-cache backend for multi-instance deployments. Behind the
//! `redis-cache` feature. The connection is established lazily on first use so
//! constructing the cache stays synchronous and a transient Redis outage degrades
//! to "cache miss" rather than failing the request.

use redis::aio::ConnectionManager;
use tokio::sync::OnceCell;

use super::CachedResponse;

pub struct RedisCache {
    url: String,
    ttl_secs: u64,
    conn: OnceCell<ConnectionManager>,
}

impl RedisCache {
    pub fn new(url: String, ttl_secs: u64) -> Self {
        Self {
            url,
            ttl_secs,
            conn: OnceCell::new(),
        }
    }

    async fn conn(&self) -> Option<ConnectionManager> {
        self.conn
            .get_or_try_init(|| async {
                redis::Client::open(self.url.as_str())?
                    .get_connection_manager()
                    .await
            })
            .await
            .map_err(|e| tracing::warn!(error = %e, "redis cache connect failed; serving as miss"))
            .ok()
            .cloned()
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        let mut conn = self.conn().await?;
        let bytes: Option<Vec<u8>> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut conn)
            .await
            .ok()?;
        bytes.and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub async fn set(&self, key: String, value: CachedResponse) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&value) else {
            return;
        };
        let _: Result<(), _> = redis::cmd("SET")
            .arg(key)
            .arg(bytes)
            .arg("EX")
            .arg(self.ttl_secs)
            .query_async(&mut conn)
            .await;
    }
}
