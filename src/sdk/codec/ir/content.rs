//! Content blocks and message types shared across all codecs.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Image reference. Protocols disagree on inline-base64 vs URL, so we keep both.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
    },
    /// Assistant asking to call a tool.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// User-supplied result of a previous tool call.
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
    },
    /// Extended-thinking / reasoning text.
    Thinking {
        text: String,
        signature: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Role {
    pub fn as_anthropic(&self) -> &'static str {
        match self {
            // Anthropic has no top-level system role inside `messages`; callers
            // hoist system blocks out. User/Tool both map to "user" turns.
            Self::System | Self::User | Self::Tool => "user",
            Self::Assistant => "assistant",
        }
    }
}
