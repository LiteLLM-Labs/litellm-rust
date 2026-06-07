//! Cache replay and store helpers for the request pipeline.

use std::{pin::Pin, sync::Arc};

use axum::{
    http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};

use crate::{
    http::llm,
    proxy::{cache::CachedResponse, state::AppState},
};

/// Content-type to record for a cached response, defaulting to JSON.
pub(super) fn content_type_of(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_owned()
}

/// Reconstruct an HTTP response from a cache hit (never calls the upstream).
/// `tag` is echoed in `x-litellm-cache` (`hit` for exact-match, `semantic`).
pub(super) fn replay_cached(cached: CachedResponse, tag: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(&cached.content_type) {
        headers.insert(CONTENT_TYPE, ct);
    }
    headers.insert("x-litellm-cache", HeaderValue::from_static(tag));
    let status = StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK);
    if cached.is_stream {
        let body = cached.body;
        let stream =
            futures_util::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(body)) });
        llm::build_stream_response(status, headers, stream)
    } else {
        llm::build_bytes_response(status, headers, cached.body)
    }
}

/// Store a fully-buffered (non-streaming) response into the exact-match cache
/// and/or record it in the semantic cache (`(scope, query_text)`).
pub(super) async fn store_response(
    state: &Arc<AppState>,
    exact_key: Option<String>,
    semantic: Option<(&str, &str)>,
    status: u16,
    content_type: String,
    body: Vec<u8>,
) {
    let cached = CachedResponse {
        status,
        content_type,
        body,
        is_stream: false,
    };
    // Record the semantic entry off the request path: embedding the query can hit
    // a remote model, and neither the client response nor the exact-cache write
    // should wait on (or be delayed by) a slow/timing-out embeddings endpoint.
    if let Some((scope, text)) = semantic {
        let state = state.clone();
        let scope = scope.to_owned();
        let text = text.to_owned();
        let cached = cached.clone();
        tokio::spawn(async move {
            state.semantic.record(&scope, &text, cached).await;
        });
    }
    if let Some(key) = exact_key {
        state.cache.set(key, cached).await;
    }
}

/// Wrap a byte stream so it forwards each chunk to the client while buffering the
/// full body; on clean completion the buffer is stored (spawned, never blocking
/// the client). A stream that errors mid-flight, or whose body exceeds
/// `max_bytes`, is forwarded but never stored (bounds memory).
/// In-flight state for [`tee_and_store`]: the wrapped stream plus the buffer and
/// metadata needed to store the body once the stream completes cleanly.
struct Tee {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    acc: Vec<u8>,
    /// Set when the stream errored or outgrew `max_bytes`; suppresses storing.
    abort_store: bool,
    done: bool,
    max_bytes: usize,
    state: Arc<AppState>,
    key: String,
    status: u16,
    content_type: String,
}

impl Tee {
    /// Buffer a forwarded chunk, giving up buffering once it would exceed the cap.
    fn buffer(&mut self, chunk: &Bytes) {
        if self.abort_store {
            return;
        }
        if self.acc.len() + chunk.len() > self.max_bytes {
            self.abort_store = true;
            self.acc = Vec::new();
        } else {
            self.acc.extend_from_slice(chunk);
        }
    }

    /// On clean completion, spawn a task to store the buffered body (never blocks
    /// the client). No-op when storing was aborted or the body is empty.
    fn spawn_store(&mut self) {
        if self.abort_store || self.acc.is_empty() {
            return;
        }
        let body = std::mem::take(&mut self.acc);
        let state = self.state.clone();
        let key = std::mem::take(&mut self.key);
        let content_type = std::mem::take(&mut self.content_type);
        let status = self.status;
        let bytes = body.len();
        tokio::spawn(async move {
            let cached = CachedResponse {
                status,
                content_type,
                body,
                is_stream: true,
            };
            state.cache.set(key, cached).await;
            tracing::trace!(bytes, "stored streaming response in cache");
        });
    }
}

/// Wrap a byte stream so it forwards each chunk to the client while buffering the
/// full body; on clean completion the buffer is stored (spawned, never blocking
/// the client). A stream that errors mid-flight, or whose body exceeds
/// `max_bytes`, is forwarded but never stored (bounds memory).
pub(super) fn tee_and_store(
    state: Arc<AppState>,
    key: String,
    status: u16,
    content_type: String,
    max_bytes: u64,
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let st = Tee {
        inner,
        acc: Vec::new(),
        abort_store: false,
        done: false,
        max_bytes: max_bytes.try_into().unwrap_or(usize::MAX),
        state,
        key,
        status,
        content_type,
    };
    futures_util::stream::unfold(st, |mut st| async move {
        if st.done {
            return None;
        }
        match st.inner.next().await {
            Some(Ok(chunk)) => {
                st.buffer(&chunk);
                Some((Ok(chunk), st))
            }
            Some(Err(e)) => {
                st.abort_store = true;
                Some((Err(e), st))
            }
            None => {
                st.done = true;
                st.spawn_store();
                None
            }
        }
    })
}
