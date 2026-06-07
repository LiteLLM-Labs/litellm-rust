use serde::Deserialize;

/// Exact-match response cache. Disabled by default; when on, an identical request
/// returns the stored response without calling the upstream (0 tokens).
#[derive(Debug, Clone, Deserialize)]
pub struct CacheSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub backend: CacheBackendKind,
    /// Required when `backend = redis`; env-expandable.
    pub redis_url: Option<String>,
    /// File path for the `redb` backend; env-expandable. Defaults to
    /// `litellm-cache.redb` when `backend = redb` and this is unset.
    #[serde(default)]
    pub redb_path: Option<String>,
    #[serde(default = "default_cache_ttl")]
    pub ttl_secs: u64,
    /// Entry-count cap (memory: immediate; redb: soft, reconciled by a periodic
    /// sweep; Redis: ignored, TTL only).
    #[serde(default = "default_cache_max_entries")]
    pub max_entries: u64,
    /// Cache requests with `temperature > 0` (non-deterministic) too. Off by
    /// default so only deterministic requests are cached.
    #[serde(default)]
    pub cache_non_deterministic: bool,
    /// Buffer and replay streaming (SSE) responses. On by default.
    #[serde(default = "default_true")]
    pub cache_streaming: bool,
    /// Max bytes buffered for a single streaming response; if a stream exceeds
    /// this it is forwarded to the client but not cached (bounds memory).
    #[serde(default = "default_max_stream_bytes")]
    pub max_stream_bytes: u64,
    /// Include a hash of the caller's API key in the cache key so tenants never
    /// see each other's cached responses. On by default.
    #[serde(default = "default_true")]
    pub scope_by_api_key: bool,
    #[serde(default)]
    pub semantic: SemanticCacheSettings,
}

/// Embedding-based semantic cache (feature `semantic-cache`). ⚠️ Off by default
/// and not recommended for coding-agent workloads (low hit rate, wrong-answer
/// risk). Restricted to deterministic, tool-free, non-streaming requests.
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticCacheSettings {
    #[serde(default)]
    pub enabled: bool,
    /// OpenAI-compatible embeddings endpoint base (env-expandable).
    pub embedding_api_base: Option<String>,
    /// API key for the embeddings endpoint (env-expandable).
    pub embedding_api_key: Option<String>,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Cosine similarity above which a cached response is reused (0..1).
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
    /// Max in-process embedding entries (LRU).
    #[serde(default = "default_semantic_max_entries")]
    pub max_entries: u64,
    /// Skip prompts longer than this (bounds embedding cost).
    #[serde(default = "default_semantic_max_chars")]
    pub max_chars: u64,
}

fn default_embedding_model() -> String {
    "text-embedding-3-small".to_owned()
}
fn default_similarity_threshold() -> f32 {
    0.92
}
fn default_semantic_max_entries() -> u64 {
    1000
}
fn default_semantic_max_chars() -> u64 {
    8000
}

impl Default for SemanticCacheSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            embedding_api_base: None,
            embedding_api_key: None,
            embedding_model: default_embedding_model(),
            similarity_threshold: default_similarity_threshold(),
            max_entries: default_semantic_max_entries(),
            max_chars: default_semantic_max_chars(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CacheBackendKind {
    #[default]
    Memory,
    Redb,
    Redis,
}

fn default_cache_ttl() -> u64 {
    300
}
fn default_cache_max_entries() -> u64 {
    10_000
}
fn default_max_stream_bytes() -> u64 {
    8 * 1024 * 1024
}
fn default_true() -> bool {
    true
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: CacheBackendKind::Memory,
            redis_url: None,
            redb_path: None,
            ttl_secs: default_cache_ttl(),
            max_entries: default_cache_max_entries(),
            cache_non_deterministic: false,
            cache_streaming: true,
            max_stream_bytes: default_max_stream_bytes(),
            scope_by_api_key: true,
            semantic: SemanticCacheSettings::default(),
        }
    }
}
