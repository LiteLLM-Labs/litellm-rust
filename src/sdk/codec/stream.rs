//! Streaming primitives shared by every codec: an incremental SSE decoder, plus
//! the stateful parser/renderer traits that bridge a provider's wire stream to
//! the canonical `StreamEvent` sequence and back out to the client's protocol.

use crate::{errors::GatewayError, sdk::codec::ir::StreamEvent};

/// One decoded Server-Sent Event. `data` is the concatenation of all `data:`
/// lines in the block (joined by `\n`), with the leading space stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental SSE decoder. Feed it raw upstream bytes; it yields complete
/// events as they cross blank-line boundaries and buffers the remainder.
///
/// Buffers raw **bytes**, not a lossily-decoded string: a multi-byte UTF-8
/// character (emoji/CJK) split across HTTP chunks must be reassembled before
/// decoding, or `from_utf8_lossy` would replace the partial bytes with U+FFFD.
/// Block boundaries are pure ASCII (`\n`/`\r`), which never collide with UTF-8
/// continuation bytes, so scanning the raw buffer is safe.
#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, consumed)) = next_boundary(&self.buf) {
            // The block ends on a boundary, so its bytes are complete; lossy
            // decoding here can't split a multi-byte character. `str::lines`
            // strips any residual `\r` from CRLF line endings.
            let block = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            self.buf.drain(..consumed);
            if let Some(event) = parse_block(&block) {
                events.push(event);
            }
        }
        events
    }
}

/// Find the first blank-line block boundary, returning `(block_end, consumed)`
/// where `consumed` includes the terminator. Handles `\n\n` and `\r\n\r\n`.
fn next_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n");
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(a), Some(b)) if a <= b => Some((a, a + 2)),
        (Some(_), Some(b)) => Some((b, b + 4)),
        (Some(a), None) => Some((a, a + 2)),
        (None, Some(b)) => Some((b, b + 4)),
        (None, None) => None,
    }
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in block.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue; // blank line or comment
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim_start().to_owned());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // other fields (id:, retry:) are ignored
    }
    if event.is_none() && data_lines.is_empty() {
        return None;
    }
    Some(SseEvent {
        event,
        data: data_lines.join("\n"),
    })
}

/// Parses a provider's wire SSE stream into canonical `StreamEvent`s. Stateful:
/// implementations accumulate cross-event context (block indices, tool-call ids).
pub trait StreamParser: Send {
    fn push(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, GatewayError>;
    /// Called once after the upstream stream ends. Emits any trailing events.
    fn finish(&mut self) -> Vec<StreamEvent> {
        Vec::new()
    }
}

/// Renders canonical `StreamEvent`s into the client protocol's SSE bytes.
/// Stateful for the same reasons as `StreamParser`.
pub trait StreamRenderer: Send {
    fn push(&mut self, event: &StreamEvent) -> Vec<u8>;
    /// Called once after all events; emits trailing frames (e.g. `[DONE]`).
    fn finish(&mut self) -> Vec<u8> {
        Vec::new()
    }
}

/// Helper to format one SSE frame with an optional event name.
pub fn sse_frame(event: Option<&str>, data: &str) -> Vec<u8> {
    let mut out = String::new();
    if let Some(name) = event {
        out.push_str("event: ");
        out.push_str(name);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(data);
    out.push_str("\n\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_events_across_chunk_boundaries() {
        let mut dec = SseDecoder::new();
        let mut events = dec.push(b"event: message_start\ndata: {\"a\":");
        assert!(events.is_empty()); // incomplete block buffered
        events = dec.push(b"1}\n\nevent: ping\ndata: {}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("message_start"));
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[1].event.as_deref(), Some("ping"));
    }

    #[test]
    fn reassembles_utf8_split_across_chunks() {
        let mut dec = SseDecoder::new();
        let bytes = "data: 🎉\n\n".as_bytes().to_vec();
        // Split one byte into the 4-byte emoji (after "data: ").
        let events = dec.push(&bytes[..7]);
        assert!(events.is_empty());
        let events = dec.push(&bytes[7..]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "🎉");
    }

    #[test]
    fn handles_crlf_and_done_sentinel() {
        let mut dec = SseDecoder::new();
        let events = dec.push(b"data: [DONE]\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "[DONE]");
    }

    #[test]
    fn trailing_block_without_terminator_is_dropped() {
        // Accepted boundary: if the upstream stream does not end with a "\n\n"
        // terminator, the residual buffer is dropped. SseDecoder yields events
        // only across "\n\n" boundaries and has no finish(), so a trailing block
        // never crosses a boundary and is discarded when the stream ends.
        // Conformant providers always terminate their final event correctly.
        let mut dec = SseDecoder::new();

        // First chunk has no "\n\n" terminator: the block is buffered, not emitted.
        let events = dec.push(b"event: x\ndata: {\"a\":1}");
        assert!(events.is_empty());

        // A second chunk that still lacks a terminator keeps the block buffered;
        // without a "\n\n" the trailing block is never produced and, since
        // SseDecoder has no finish(), it is dropped when the stream ends.
        let events = dec.push(b"more: still-no-terminator");
        assert!(events.is_empty());
    }
}
