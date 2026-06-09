//! Normalized streaming-event IR.

use super::response::{StopReason, Usage};

/// What kind of content block a stream is opening.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockStart {
    Text,
    Thinking,
    ToolUse { id: String, name: String },
}

/// Normalized streaming events. Mirrors Anthropic's SSE shape (the richest of
/// the four) so any protocol's stream can be reconstructed from this sequence.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    MessageStart {
        id: String,
        model: String,
    },
    ContentBlockStart {
        index: usize,
        block: BlockStart,
    },
    TextDelta {
        index: usize,
        text: String,
    },
    ThinkingDelta {
        index: usize,
        text: String,
    },
    /// Partial JSON for a `ToolUse` block's `input`.
    ToolUseInputDelta {
        index: usize,
        partial_json: String,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        stop_reason: Option<StopReason>,
        usage: Option<Usage>,
    },
    MessageStop,
}
