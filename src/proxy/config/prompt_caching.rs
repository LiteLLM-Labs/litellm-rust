use serde::Deserialize;

/// Gateway-side prompt-cache (Anthropic breakpoint) policy. Disabled by default
/// so the request path is unchanged unless an operator opts in.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptCachingSettings {
    /// Master switch for any gateway-side prompt-cache handling.
    #[serde(default)]
    pub enabled: bool,
    /// Auto-inject breakpoints for clients that didn't set `cache_control` when
    /// the request is routed to an Anthropic upstream. Off by default because it
    /// assumes a stable system/tools prefix (true for agent loops); on a volatile
    /// prefix it can cost more than it saves.
    #[serde(default)]
    pub auto_inject: bool,
    /// Max breakpoints to inject (clamped to Anthropic's hard cap of 4).
    #[serde(default = "default_max_breakpoints")]
    pub max_breakpoints: u8,
    /// Minimum estimated tokens a cached prefix must reach to be worth a
    /// breakpoint (Anthropic ignores prefixes below ~1024 tokens).
    #[serde(default = "default_min_tokens")]
    pub min_tokens: u64,
    /// Chars-per-token divisor for the size estimate (no tokenizer is run).
    #[serde(default = "default_chars_per_token")]
    pub chars_per_token: u64,
}

fn default_max_breakpoints() -> u8 {
    4
}
fn default_min_tokens() -> u64 {
    1024
}
fn default_chars_per_token() -> u64 {
    4
}

impl Default for PromptCachingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_inject: false,
            max_breakpoints: default_max_breakpoints(),
            min_tokens: default_min_tokens(),
            chars_per_token: default_chars_per_token(),
        }
    }
}
