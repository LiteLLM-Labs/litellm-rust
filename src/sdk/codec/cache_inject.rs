//! Gateway-side auto-injection of Anthropic prompt-cache breakpoints.
//!
//! Clients that can't express `cache_control` (OpenAI/Gemini wire formats) still
//! benefit from Anthropic prompt caching when routed to an Anthropic upstream —
//! but only if the gateway places the breakpoints. This is a pure function over
//! the IR so it stays codec-agnostic and unit-testable; the pipeline calls it on
//! the cross-protocol path when the outbound wire is Anthropic and the operator
//! opted in.
//!
//! It is best-effort: with no tokenizer we estimate sizes from character counts
//! and only mark a prefix once the cumulative estimate clears Anthropic's minimum
//! cacheable size (under-counting just skips a breakpoint, which is safe).

use crate::sdk::codec::ir::{ChatRequest, ContentBlock, ImageSource, Message, ToolDef};

/// Anthropic's hard cap on cache breakpoints per request.
const ANTHROPIC_MAX_BREAKPOINTS: usize = 4;
/// Above this many messages a single tail breakpoint can fall outside Anthropic's
/// 20-block read lookback, so we add a second anchor mid-history.
const LONG_HISTORY_MESSAGES: usize = 20;

/// Inject ephemeral cache breakpoints on the stable prefix (tools → system →
/// leading messages) when the client set none. No-op if the client already set
/// any breakpoint (we never stack on top of client intent).
pub fn auto_inject_anthropic_breakpoints(
    req: &mut ChatRequest,
    max_breakpoints: usize,
    min_tokens: u64,
    chars_per_token: u64,
) {
    if !req.cache.is_empty() {
        return;
    }
    let mut budget = max_breakpoints.min(ANTHROPIC_MAX_BREAKPOINTS);
    if budget == 0 {
        return;
    }
    let cpt = chars_per_token.max(1);
    let est = |chars: usize| chars as u64 / cpt;

    // Cumulative prefix grows in Anthropic's hierarchy: tools, then system, then
    // messages. A breakpoint only pays off once the cached prefix clears the
    // minimum; mark the tightest layer whose own cumulative prefix qualifies.
    let mut cumulative = req.tools.iter().map(tool_chars).sum::<usize>();
    if !req.tools.is_empty() && budget > 0 && est(cumulative) >= min_tokens {
        req.cache.tools = true;
        budget -= 1;
    }

    cumulative += req.system.iter().map(block_chars).sum::<usize>();
    if !req.system.is_empty() && budget > 0 && est(cumulative) >= min_tokens {
        req.cache.system = true;
        budget -= 1;
    }

    cumulative += req.messages.iter().map(msg_chars).sum::<usize>();
    if !req.messages.is_empty() && budget > 0 && est(cumulative) >= min_tokens {
        let last = req.messages.len() - 1;
        // For long histories, drop an earlier anchor first so it stays within the
        // 20-block lookback window (kept ascending for deterministic ordering).
        if req.messages.len() > LONG_HISTORY_MESSAGES && budget >= 2 {
            req.cache.messages.push(req.messages.len() / 2);
            budget -= 1;
        }
        if budget > 0 {
            req.cache.messages.push(last);
        }
    }
}

fn block_chars(b: &ContentBlock) -> usize {
    match b {
        ContentBlock::Text { text } | ContentBlock::Thinking { text, .. } => text.len(),
        ContentBlock::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
        ContentBlock::ToolResult { content, .. } => content.iter().map(block_chars).sum(),
        ContentBlock::Image { source } => match source {
            ImageSource::Base64 { data, .. } => data.len(),
            ImageSource::Url(u) => u.len(),
        },
    }
}

fn tool_chars(t: &ToolDef) -> usize {
    t.name.len() + t.description.as_ref().map_or(0, String::len) + t.parameters.to_string().len()
}

fn msg_chars(m: &Message) -> usize {
    m.content.iter().map(block_chars).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk::codec::ir::{CacheMarkers, ContentBlock, Message, Role, ToolDef};
    use serde_json::json;

    fn text_block(n: usize) -> ContentBlock {
        ContentBlock::Text {
            text: "x".repeat(n),
        }
    }
    fn user_msg(n: usize) -> Message {
        Message {
            role: Role::User,
            content: vec![text_block(n)],
        }
    }
    fn tool(n: usize) -> ToolDef {
        ToolDef {
            name: "x".repeat(n),
            description: None,
            parameters: json!({"type": "object"}),
            builtin: None,
        }
    }

    #[test]
    fn skips_when_below_threshold() {
        let mut req = ChatRequest {
            system: vec![text_block(5)],
            messages: vec![user_msg(5)],
            ..Default::default()
        };
        // chars_per_token=1, min_tokens=100 → 10 chars total is far below.
        auto_inject_anthropic_breakpoints(&mut req, 4, 100, 1);
        assert!(req.cache.is_empty());
    }

    #[test]
    fn marks_tools_system_and_tail_message() {
        let mut req = ChatRequest {
            tools: vec![tool(50)],
            system: vec![text_block(50)],
            messages: vec![user_msg(50), user_msg(50)],
            ..Default::default()
        };
        auto_inject_anthropic_breakpoints(&mut req, 4, 10, 1);
        assert!(req.cache.tools);
        assert!(req.cache.system);
        assert_eq!(req.cache.messages, vec![1]); // tail message
    }

    #[test]
    fn never_stacks_on_client_breakpoints() {
        let mut req = ChatRequest {
            system: vec![text_block(500)],
            cache: CacheMarkers {
                system: true,
                ..Default::default()
            },
            ..Default::default()
        };
        auto_inject_anthropic_breakpoints(&mut req, 4, 1, 1);
        // unchanged: still just the client's system marker, no tools/messages added
        assert!(req.cache.system);
        assert!(!req.cache.tools);
        assert!(req.cache.messages.is_empty());
    }

    #[test]
    fn respects_breakpoint_budget() {
        let mut req = ChatRequest {
            tools: vec![tool(50)],
            system: vec![text_block(50)],
            messages: vec![user_msg(50)],
            ..Default::default()
        };
        // budget of 1 → only the tools layer gets the single breakpoint.
        auto_inject_anthropic_breakpoints(&mut req, 1, 10, 1);
        assert!(req.cache.tools);
        assert!(!req.cache.system);
        assert!(req.cache.messages.is_empty());
    }

    #[test]
    fn adds_anchor_for_long_history() {
        let mut req = ChatRequest {
            messages: (0..30).map(|_| user_msg(50)).collect(),
            ..Default::default()
        };
        auto_inject_anthropic_breakpoints(&mut req, 4, 10, 1);
        // mid-history anchor + tail.
        assert_eq!(req.cache.messages, vec![15, 29]);
    }
}
