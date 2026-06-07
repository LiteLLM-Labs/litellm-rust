//! Optional embedding-based semantic response cache (feature `semantic-cache`).
//!
//! ⚠️ Research shows semantic caching is net-negative for coding-agent workloads
//! (5-20% hit rate, and a wrong-answer hit on code is actively harmful), so it is
//! default-off and opt-in. It is further restricted to deterministic, tool-free,
//! non-streaming requests, partitioned by tenant, and fails open (any embedding
//! error degrades to a normal upstream call).
//!
//! Without the `semantic-cache` feature this compiles to a no-op `Disabled`.

use serde_json::Value;

use crate::proxy::{cache::CachedResponse, config::SemanticCacheSettings};

pub enum SemanticCache {
    Disabled,
    #[cfg(feature = "semantic-cache")]
    Enabled(inner::Inner),
}

impl std::fmt::Debug for SemanticCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Disabled => "Disabled",
            #[cfg(feature = "semantic-cache")]
            Self::Enabled(_) => "Enabled",
        };
        write!(f, "SemanticCache::{name}")
    }
}

impl SemanticCache {
    pub fn from_config(settings: &SemanticCacheSettings, _http: reqwest::Client) -> Self {
        if !settings.enabled {
            return Self::Disabled;
        }
        #[cfg(feature = "semantic-cache")]
        {
            Self::Enabled(inner::Inner::new(settings, _http))
        }
        #[cfg(not(feature = "semantic-cache"))]
        {
            tracing::warn!(
                "cache.semantic.enabled is set but the binary was built without \
                 --features semantic-cache; semantic cache stays disabled"
            );
            Self::Disabled
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub async fn lookup(&self, _scope: &str, _text: &str) -> Option<CachedResponse> {
        match self {
            Self::Disabled => None,
            #[cfg(feature = "semantic-cache")]
            Self::Enabled(inner) => inner.lookup(_scope, _text).await,
        }
    }

    pub async fn record(&self, _scope: &str, _text: &str, _response: CachedResponse) {
        match self {
            Self::Disabled => {}
            #[cfg(feature = "semantic-cache")]
            Self::Enabled(inner) => inner.record(_scope, _text, _response).await,
        }
    }
}

/// Whether a request is eligible for semantic caching: tool-free (tool-calling is
/// too input-sensitive to risk a near-match) and within the prompt-size cap.
pub fn eligible(body: &Value, settings: &SemanticCacheSettings) -> bool {
    let has_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    !has_tools && (query_text(body).len() as u64) <= settings.max_chars
}

/// Flatten all string content from a request body (across protocols) into a
/// single blob to embed. Includes role labels etc., but consistently, which is
/// fine for a similarity signal.
pub fn query_text(body: &Value) -> String {
    let mut out = String::new();
    for key in ["system", "instructions", "messages", "contents", "input"] {
        collect_strings(body.get(key), &mut out);
    }
    out
}

fn collect_strings(v: Option<&Value>, out: &mut String) {
    match v {
        Some(Value::String(s)) => {
            out.push_str(s);
            out.push('\n');
        }
        Some(Value::Array(a)) => a.iter().for_each(|x| collect_strings(Some(x), out)),
        Some(Value::Object(o)) => o.values().for_each(|x| collect_strings(Some(x), out)),
        _ => {}
    }
}

#[cfg(feature = "semantic-cache")]
mod inner {
    use std::{collections::VecDeque, sync::Mutex};

    use reqwest::{header::AUTHORIZATION, Client};
    use serde_json::json;

    use super::CachedResponse;
    use crate::proxy::config::SemanticCacheSettings;

    struct Entry {
        scope: String,
        embedding: Vec<f32>,
        response: CachedResponse,
    }

    pub struct Inner {
        http: Client,
        api_base: String,
        auth_token: String,
        model: String,
        threshold: f32,
        max_entries: usize,
        store: Mutex<VecDeque<Entry>>,
    }

    impl Inner {
        pub fn new(s: &SemanticCacheSettings, http: Client) -> Self {
            Self {
                http,
                api_base: s.embedding_api_base.clone().unwrap_or_default(),
                auth_token: s.embedding_api_key.clone().unwrap_or_default(),
                model: s.embedding_model.clone(),
                threshold: s.similarity_threshold,
                max_entries: (s.max_entries as usize).max(1),
                store: Mutex::new(VecDeque::new()),
            }
        }

        /// Embed text via an OpenAI-compatible endpoint. Fails open (returns None).
        async fn embed(&self, text: &str) -> Option<Vec<f32>> {
            let url = format!("{}/v1/embeddings", self.api_base.trim_end_matches('/'));
            let mut req = self
                .http
                .post(url)
                .timeout(std::time::Duration::from_secs(5))
                .json(&json!({"model": self.model, "input": text}));
            if !self.auth_token.is_empty() {
                let scheme = "Bearer";
                req = req.header(AUTHORIZATION, format!("{scheme} {}", self.auth_token));
            }
            let resp = req.send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let v: serde_json::Value = resp.json().await.ok()?;
            let arr = v.get("data")?.get(0)?.get("embedding")?.as_array()?;
            Some(
                arr.iter()
                    .filter_map(|x| x.as_f64().map(|f| f as f32))
                    .collect(),
            )
        }

        pub async fn lookup(&self, scope: &str, text: &str) -> Option<CachedResponse> {
            let q = self.embed(text).await?;
            let store = self.store.lock().ok()?;
            let mut best: Option<(f32, &CachedResponse)> = None;
            for e in store.iter().filter(|e| e.scope == scope) {
                let sim = cosine(&q, &e.embedding);
                if sim >= self.threshold && best.is_none_or(|(b, _)| sim > b) {
                    best = Some((sim, &e.response));
                }
            }
            best.map(|(_, r)| r.clone())
        }

        pub async fn record(&self, scope: &str, text: &str, response: CachedResponse) {
            let Some(embedding) = self.embed(text).await else {
                return;
            };
            let Ok(mut store) = self.store.lock() else {
                return;
            };
            if store.len() >= self.max_entries {
                store.pop_front();
            }
            store.push_back(Entry {
                scope: scope.to_owned(),
                embedding,
                response,
            });
        }
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
        for (x, y) in a.iter().zip(b) {
            dot += x * y;
            na += x * x;
            nb += y * y;
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        dot / (na.sqrt() * nb.sqrt())
    }

    #[cfg(test)]
    mod tests {
        use super::cosine;

        #[test]
        fn cosine_identical_is_one_orthogonal_is_zero() {
            assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
            assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
            assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0); // length mismatch
        }
    }
}
