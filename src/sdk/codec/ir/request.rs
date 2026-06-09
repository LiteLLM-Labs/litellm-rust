//! Request-side IR: the canonical `ChatRequest` and its knobs.

use serde_json::{Map, Value};

use super::content::{ContentBlock, Message};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema object for the tool's parameters.
    pub parameters: Value,
    /// For provider built-in / server-side tools (web search, code execution,
    /// …), the verbatim native tool entry. `None` for ordinary function tools.
    /// Built-ins are dropped on cross-protocol render rather than mangled into a
    /// bogus function tool.
    pub builtin: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Tool(String),
}

impl ToolChoice {
    /// Whether this choice should be forwarded given the function tools that
    /// survived rendering. A named choice targeting a tool absent from the
    /// rendered set (e.g. a built-in that was filtered out) must be dropped, or
    /// the provider rejects the request for referencing a nonexistent tool.
    pub fn applies_to(&self, function_names: &[&str]) -> bool {
        match self {
            ToolChoice::Tool(name) => function_names.contains(&name.as_str()),
            _ => true,
        }
    }
}

/// Requested structured-output format.
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseFormat {
    /// Any valid JSON object.
    JsonObject,
    /// JSON constrained to a schema.
    JsonSchema {
        name: String,
        schema: Value,
        strict: bool,
    },
}

/// Reasoning / extended-thinking request knob. Both forms are carried when known
/// so each codec can pick the closest native shape without double-converting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReasoningConfig {
    pub effort: Option<Effort>,
    pub budget_tokens: Option<u64>,
}

impl ReasoningConfig {
    /// The effort tier, deriving it from a token budget when only that is known.
    pub fn derived_effort(&self) -> Effort {
        self.effort
            .unwrap_or_else(|| Effort::from_budget(self.budget_tokens.unwrap_or(0)))
    }

    /// A token budget, deriving it from the effort tier when only that is known.
    pub fn derived_budget(&self) -> u64 {
        self.budget_tokens
            .or_else(|| self.effort.map(|e| e.to_budget()))
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" | "none" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Heuristic token budget for protocols that take a number instead of a tier.
    pub fn to_budget(&self) -> u64 {
        match self {
            Self::Minimal => 1024,
            Self::Low => 4096,
            Self::Medium => 8192,
            Self::High => 16384,
        }
    }

    pub fn from_budget(budget: u64) -> Self {
        match budget {
            0..=1024 => Self::Minimal,
            1025..=4096 => Self::Low,
            4097..=8192 => Self::Medium,
            _ => Self::High,
        }
    }
}

/// Prompt-cache breakpoints for a request, kept out-of-band so the content-block
/// types stay untouched. Anthropic breakpoints always sit at a prefix boundary
/// (end of tools / end of system / the tail block of a message), so marking the
/// carrier rather than an individual block covers real usage; a breakpoint in the
/// middle of a message collapses to that message's last block on render.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheMarkers {
    /// Breakpoint on the last tool definition (caches the whole `tools` prefix).
    pub tools: bool,
    /// Breakpoint on the last system block (caches `tools` + `system`).
    pub system: bool,
    /// Indices into `messages` whose tail block carries a breakpoint.
    pub messages: Vec<usize>,
}

impl CacheMarkers {
    pub fn is_empty(&self) -> bool {
        !self.tools && !self.system && self.messages.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    /// System / developer instructions, usually a single `Text` block.
    pub system: Vec<ContentBlock>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub tool_choice: Option<ToolChoice>,
    /// Prompt-cache breakpoints. Empty unless the client set `cache_control` or
    /// the gateway auto-injected them. Only honoured when rendering to Anthropic.
    pub cache: CacheMarkers,
    /// `Some(false)` forbids parallel tool calls; `None` leaves it unspecified.
    pub parallel_tool_calls: Option<bool>,
    pub response_format: Option<ResponseFormat>,
    pub reasoning: Option<ReasoningConfig>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub stream: bool,
    /// Params we do not model explicitly, carried through to the outbound body
    /// when the target protocol is shape-compatible (best effort).
    pub extra: Map<String, Value>,
}
