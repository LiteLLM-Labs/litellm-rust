//! Embedded pure-Rust persistent cache (redb). No daemon; survives restart.
//! Sync API wrapped in `spawn_blocking`. TTL is enforced exactly on read (each
//! value stores its `expire_at`; expired entries are never served), while a
//! background sweep lazily reclaims disk. Any error degrades to "cache miss"
//! rather than failing the request.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::CachedResponse;
use crate::{errors::GatewayError, proxy::config::CacheSettings};

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("responses");
const DEFAULT_PATH: &str = "litellm-cache.redb";
const SWEEP_INTERVAL_SECS: u64 = 60;

/// On-disk value: the cached response plus its absolute expiry (unix seconds).
#[derive(Serialize, Deserialize)]
struct Stored {
    expire_at: u64,
    response: CachedResponse,
}

pub struct RedbCache {
    db: Arc<Database>,
    ttl_secs: u64,
    max_entries: u64,
    /// Serializes writes at the async layer. redb already allows only one write
    /// txn at a time; gating here means concurrent `set`s wait as cheap async
    /// tasks instead of each parking a `spawn_blocking` thread on redb's
    /// internal write condvar.
    write_lock: Arc<Mutex<()>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl RedbCache {
    /// Open (creating if needed) the database and materialize the table via a
    /// committed write txn so the first `get` can't fail on a missing table.
    /// Does not spawn any background task.
    pub fn open(path: &str, settings: &CacheSettings) -> Result<Self, GatewayError> {
        // `Database::create` fails if the parent directory is missing; create it so a
        // drop-in `disk_cache_dir` config (mapped to <dir>/litellm-cache.redb) works.
        if let Some(parent) = std::path::Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                GatewayError::InvalidConfig(format!(
                    "cannot create cache dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let db = Database::create(path).map_err(|e| {
            GatewayError::InvalidConfig(format!(
                "cache.redb_path {path} could not be opened: {e}; redb requires \
                 exclusive access, so each instance needs its own file and the \
                 path can't be shared across processes"
            ))
        })?;
        {
            let txn = db
                .begin_write()
                .map_err(|e| GatewayError::InvalidConfig(format!("redb cache init failed: {e}")))?;
            txn.open_table(TABLE)
                .map_err(|e| GatewayError::InvalidConfig(format!("redb cache init failed: {e}")))?;
            txn.commit()
                .map_err(|e| GatewayError::InvalidConfig(format!("redb cache init failed: {e}")))?;
        }
        Ok(Self {
            db: Arc::new(db),
            ttl_secs: settings.ttl_secs,
            max_entries: settings.max_entries,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Open from config (defaulting the path) and spawn the periodic sweep.
    /// Must be called from within a Tokio runtime (it spawns the sweep task).
    pub fn from_config(settings: &CacheSettings) -> Result<Self, GatewayError> {
        let path = settings.redb_path.as_deref().unwrap_or(DEFAULT_PATH);
        let cache = Self::open(path, settings)?;

        let db = cache.db.clone();
        let write_lock = cache.write_lock.clone();
        let max_entries = cache.max_entries;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
            loop {
                tick.tick().await;
                // Scan with a read txn (MVCC, no write lock) — the expensive
                // part (full iterate + deserialize + sort) never blocks `set`.
                let scan_db = db.clone();
                let removals =
                    tokio::task::spawn_blocking(move || collect_removals(&scan_db, max_entries))
                        .await
                        .unwrap_or_default();
                if removals.is_empty() {
                    continue;
                }
                // Then a short write txn under the shared write lock.
                let _guard = write_lock.lock().await;
                let rm_db = db.clone();
                let _ = tokio::task::spawn_blocking(move || remove_keys(&rm_db, &removals)).await;
            }
        });

        Ok(cache)
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        let db = self.db.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_read().ok()?;
            let table = txn.open_table(TABLE).ok()?;
            let guard = table.get(key.as_str()).ok()??;
            let stored: Stored = serde_json::from_slice(guard.value()).ok()?;
            (stored.expire_at > now_secs()).then_some(stored.response)
        })
        .await
        .ok()
        .flatten()
    }

    pub async fn set(&self, key: String, value: CachedResponse) {
        let db = self.db.clone();
        let ttl_secs = self.ttl_secs;
        // Hold the write lock across the blocking write so only one redb write
        // txn is in flight at a time (see `write_lock`).
        let _guard = self.write_lock.lock().await;
        let _ = tokio::task::spawn_blocking(move || {
            let stored = Stored {
                expire_at: now_secs().saturating_add(ttl_secs),
                response: value,
            };
            let bytes = serde_json::to_vec(&stored).ok()?;
            let txn = db.begin_write().ok()?;
            {
                let mut table = txn.open_table(TABLE).ok()?;
                table.insert(key.as_str(), bytes.as_slice()).ok()?;
            }
            txn.commit().ok()
        })
        .await;
    }
}

/// Scan the table (read txn) and return the keys to drop: every expired entry,
/// plus — if the live set still exceeds `max_entries` — the soonest-to-expire
/// live entries (FIFO by `expire_at`) down to the limit. Full-table scan;
/// acceptable for v1, a secondary index could replace it if the table grows.
fn collect_removals(db: &Database, max_entries: u64) -> Vec<String> {
    let Ok(txn) = db.begin_read() else {
        return Vec::new();
    };
    let Ok(table) = txn.open_table(TABLE) else {
        return Vec::new();
    };
    let now = now_secs();

    let mut live: Vec<(u64, String)> = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    if let Ok(iter) = table.iter() {
        for (k, v) in iter.flatten() {
            let key = k.value().to_owned();
            match serde_json::from_slice::<Stored>(v.value()) {
                Ok(s) if s.expire_at > now => live.push((s.expire_at, key)),
                _ => remove.push(key),
            }
        }
    }

    let live_count = live.len() as u64;
    if live_count > max_entries {
        let over = (live_count - max_entries) as usize;
        live.sort_unstable_by_key(|(expire_at, _)| *expire_at);
        remove.extend(live.into_iter().take(over).map(|(_, key)| key));
    }
    remove
}

/// Remove the given keys in a single short write txn.
fn remove_keys(db: &Database, keys: &[String]) {
    let Ok(txn) = db.begin_write() else {
        return;
    };
    {
        let Ok(mut table) = txn.open_table(TABLE) else {
            return;
        };
        for key in keys {
            let _ = table.remove(key.as_str());
        }
    }
    let _ = txn.commit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn settings(ttl: u64) -> CacheSettings {
        CacheSettings {
            enabled: true,
            ttl_secs: ttl,
            max_entries: 1000,
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

    /// Insert an entry with an explicit expiry, bypassing `set`'s `now + ttl`,
    /// so sweep tests can mix already-expired and long-lived entries.
    fn insert_raw(cache: &RedbCache, key: &str, expire_at: u64, body: &str) {
        let stored = Stored {
            expire_at,
            response: resp(body),
        };
        let bytes = serde_json::to_vec(&stored).unwrap();
        let txn = cache.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(TABLE).unwrap();
            table.insert(key, bytes.as_slice()).unwrap();
        }
        txn.commit().unwrap();
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.redb");
        let path = path.to_str().unwrap();

        {
            let c = RedbCache::open(path, &settings(300)).unwrap();
            c.set("k".to_owned(), resp("hello")).await;
        } // drop closes the database

        let c = RedbCache::open(path, &settings(300)).unwrap();
        assert_eq!(c.get("k").await.unwrap().body, b"hello");
    }

    #[tokio::test]
    async fn expired_entry_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.redb");
        // ttl_secs = 0 → expire_at == now, and `expire_at > now` is false.
        let c = RedbCache::open(path.to_str().unwrap(), &settings(0)).unwrap();
        c.set("k".to_owned(), resp("x")).await;
        assert!(c.get("k").await.is_none());
    }

    #[tokio::test]
    async fn sweep_drops_expired_and_enforces_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.redb");
        let c = RedbCache::open(path.to_str().unwrap(), &settings(300)).unwrap();
        let now = now_secs();

        // Two already-expired entries.
        insert_raw(&c, "exp1", now.saturating_sub(10), "a");
        insert_raw(&c, "exp2", now.saturating_sub(5), "b");
        // Three live entries with distinct expiries.
        insert_raw(&c, "live1", now + 100, "c"); // soonest-to-expire
        insert_raw(&c, "live2", now + 200, "d");
        insert_raw(&c, "live3", now + 300, "e");

        // Cap at 2: expired pruned, then 1 over → evict soonest-to-expire (live1).
        let removals = collect_removals(&c.db, 2);
        remove_keys(&c.db, &removals);

        assert!(c.get("exp1").await.is_none());
        assert!(c.get("exp2").await.is_none());
        assert!(c.get("live1").await.is_none());
        assert_eq!(c.get("live2").await.unwrap().body, b"d");
        assert_eq!(c.get("live3").await.unwrap().body, b"e");
    }
}
