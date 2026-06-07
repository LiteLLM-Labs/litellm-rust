//! Response-side IR: `ChatResponse`, `Usage`, and `StopReason` conversions.

use super::content::ContentBlock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
    ContentFilter,
    Other(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// TOTAL input tokens processed, INCLUDING `cache_read_input_tokens` and
    /// `cache_creation_input_tokens`. OpenAI/Gemini already report inclusive
    /// prompt counts; Anthropic reports only the post-breakpoint remainder, so
    /// its codec adds the cache counts back to keep this field inclusive.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Subset of `input_tokens` written to the prompt cache this turn (Anthropic,
    /// billed ~1.25x). OpenAI/Gemini have no creation concept, so 0 there.
    pub cache_creation_input_tokens: u64,
    /// Subset of `input_tokens` served from the prompt cache (billed ~0.1x).
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Input tokens billed at the full rate (total minus the cached/created
    /// portions). Saturates so a malformed upstream count can't underflow.
    pub fn non_cached_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_sub(self.cache_read_input_tokens)
            .saturating_sub(self.cache_creation_input_tokens)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<StopReason>,
    pub usage: Usage,
}

impl StopReason {
    pub fn from_anthropic(s: &str) -> Self {
        match s {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "tool_use" => Self::ToolUse,
            "stop_sequence" => Self::StopSequence,
            "refusal" => Self::ContentFilter,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn to_anthropic(&self) -> String {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::ToolUse => "tool_use",
            Self::StopSequence => "stop_sequence",
            Self::ContentFilter => "refusal",
            Self::Other(s) => s,
        }
        .to_owned()
    }

    /// OpenAI `finish_reason` value.
    pub fn from_openai(s: &str) -> Self {
        match s {
            "stop" => Self::EndTurn,
            "length" => Self::MaxTokens,
            "tool_calls" | "function_call" => Self::ToolUse,
            "content_filter" => Self::ContentFilter,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn to_openai(&self) -> String {
        match self {
            Self::EndTurn => "stop",
            Self::MaxTokens => "length",
            Self::ToolUse => "tool_calls",
            Self::StopSequence => "stop",
            Self::ContentFilter => "content_filter",
            Self::Other(s) => s,
        }
        .to_owned()
    }

    /// Gemini `finishReason` value.
    pub fn from_gemini(s: &str) -> Self {
        match s {
            "STOP" => Self::EndTurn,
            "MAX_TOKENS" => Self::MaxTokens,
            "SAFETY" | "PROHIBITED_CONTENT" => Self::ContentFilter,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn to_gemini(&self) -> String {
        match self {
            Self::EndTurn | Self::ToolUse => "STOP",
            Self::MaxTokens => "MAX_TOKENS",
            Self::StopSequence => "STOP",
            Self::ContentFilter => "SAFETY",
            Self::Other(_) => "STOP",
        }
        .to_owned()
    }
}
