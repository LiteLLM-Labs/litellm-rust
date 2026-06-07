//! SSE stream bridging between the outbound parser and inbound renderer.

use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};

use crate::sdk::codec::stream::{SseDecoder, StreamParser, StreamRenderer};

struct StreamState {
    upstream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    decoder: SseDecoder,
    parser: Box<dyn StreamParser>,
    renderer: Box<dyn StreamRenderer>,
    finished: bool,
}

/// Bridge the upstream SSE byte stream through the outbound parser and inbound
/// renderer, re-emitting the client protocol's bytes.
pub(super) fn transform_stream(
    upstream: reqwest::Response,
    parser: Box<dyn StreamParser>,
    renderer: Box<dyn StreamRenderer>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let state = StreamState {
        upstream: Box::pin(upstream.bytes_stream()),
        decoder: SseDecoder::new(),
        parser,
        renderer,
        finished: false,
    };

    futures_util::stream::unfold(state, |mut state| async move {
        loop {
            if state.finished {
                return None;
            }
            match state.upstream.next().await {
                Some(Ok(chunk)) => {
                    let mut out = Vec::new();
                    for sse in state.decoder.push(&chunk) {
                        match state.parser.push(&sse) {
                            Ok(events) => {
                                for ev in events {
                                    out.extend(state.renderer.push(&ev));
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "stream parse error"),
                        }
                    }
                    if out.is_empty() {
                        continue;
                    }
                    return Some((Ok(Bytes::from(out)), state));
                }
                Some(Err(e)) => {
                    state.finished = true;
                    return Some((Err(std::io::Error::other(e)), state));
                }
                None => {
                    let mut out = Vec::new();
                    for ev in state.parser.finish() {
                        out.extend(state.renderer.push(&ev));
                    }
                    out.extend(state.renderer.finish());
                    state.finished = true;
                    if out.is_empty() {
                        return None;
                    }
                    return Some((Ok(Bytes::from(out)), state));
                }
            }
        }
    })
}
