# Protocol translation

litellm-rust speaks four LLM wire protocols, **inbound** (the client-facing
endpoint) and **outbound** (the upstream provider), and translates between any
pair — including tool calling and streaming.

| Wire format | Inbound endpoint | Outbound URL | Tool calling |
|---|---|---|---|
| Anthropic Messages | `POST /v1/messages` | `{base}/v1/messages` | `tool_use` / `tool_result` blocks |
| OpenAI Chat Completions | `POST /v1/chat/completions` | `{base}/v1/chat/completions` | `tool_calls` / `role:tool` |
| OpenAI Responses | `POST /v1/responses` | `{base}/v1/responses` | `function_call` / `function_call_output` |
| Gemini | `POST /v1beta/models/{model}:generateContent` (`:streamGenerateContent`) | `{base}/v1beta/models/{model}:…` | `functionCall` / `functionResponse` parts |

## How it works

The **inbound protocol** is decided by the endpoint the client hits; the
**outbound protocol** is decided by the provider the model resolves to.

Translation goes through a canonical intermediate representation (IR). Each
protocol implements one [`ProtocolCodec`](../src/sdk/codec/mod.rs): it parses its
wire shape into the IR (`src/sdk/codec/ir.rs`) and renders the IR back out. So N
protocols need N codecs, not N×N point-to-point translators.

```
inbound body --[in codec].parse_request--> IR --[out codec].render_request--> upstream body
upstream resp --[out codec].parse_response--> IR --[in codec].render_response--> client body
```

Streaming works the same way per SSE event: the outbound codec's stream parser
turns the upstream SSE into canonical `StreamEvent`s, and the inbound codec's
stream renderer turns those back into the client protocol's SSE.

**Fast path.** When the inbound and outbound protocols match (e.g. a Claude
client to an Anthropic upstream), the body is passed through byte-for-byte — only
the model name and auth headers are rewritten, exactly as before. Cross-protocol
requests pay the parse/render cost; same-protocol requests do not.

## Selecting the outbound protocol

Provider id picks a default wire format:

| Provider id | Default wire format |
|---|---|
| `anthropic` | Anthropic Messages |
| `openai`, `codex` | OpenAI Responses |
| `openai_chat` | OpenAI Chat Completions |
| `gemini` | Gemini |

Override per model with `litellm_params.wire_api` (`anthropic` \| `chat` \|
`responses` \| `gemini`). For example, to send an OpenAI-shaped request to an
Anthropic upstream, map a model to `anthropic/<model>` and call
`POST /v1/chat/completions` with that model name. To force the `openai` id onto
Chat Completions instead of its Responses default, set `wire_api: chat`.

See `config.yaml.example` for ready-to-copy entries.

The Gemini endpoint accepts the gateway key via `x-goog-api-key`, `?key=`, or a
bearer token, matching the Gemini SDKs.

## Advanced feature mapping

Beyond text and function calling, these features map across protocols. Each is
additive and feature-detected: when a request doesn't use it, nothing changes,
and when a target provider can't honor it, it's dropped rather than forced
(a "stripped" provider never gets a field that would make it 400).

| Feature | Anthropic | OpenAI Chat | OpenAI Responses | Gemini |
|---|---|---|---|---|
| **Structured output** | `output_config.format` (parse only) | `response_format` | `text.format` | `responseMimeType` + `responseJsonSchema` |
| **Reasoning effort/budget** | `thinking.budget_tokens` | `reasoning_effort` | `reasoning.effort` | `thinkingConfig.thinkingBudget` |
| **Parallel tool calls** | `tool_choice.disable_parallel_tool_use` | `parallel_tool_calls` | `parallel_tool_calls` | — (no toggle) |
| **Built-in / server tools** | `web_search_20250305`, … | — | `web_search`, `file_search`, … | `google_search`, `code_execution`, … |

Notes:

- **Built-in tools** (web search, code execution, file search) are recognized
  and carried as opaque entries, then **dropped on cross-protocol render** — the
  target model answers from its own knowledge instead of being handed a fake
  client function it can't satisfy. Same-protocol requests keep them verbatim via
  the fast path.
- **Structured output**: JSON-schema requests translate between the three
  schema-native protocols. Anthropic has no stable server-side equivalent, so
  rendering to Anthropic drops it (the model is still steered by the prompt).
- **Reasoning**: effort tiers and token budgets convert heuristically. Rendering
  to Anthropic clamps `budget_tokens` below `max_tokens` and omits a custom
  sampling temperature, both of which Anthropic rejects with extended thinking.

## Known lossy edges

Cross-protocol translation is best-effort where protocols have no equivalent:

- **Thinking / reasoning text.** Anthropic `thinking`, Responses `reasoning`, and
  Gemini `thought` parts map across; OpenAI Chat has no native field, so they
  surface as `reasoning_content` (non-standard) or are dropped.
- **Reasoning signatures.** The opaque blobs that authenticate reasoning
  (Anthropic `signature`, Responses `encrypted_content`, Gemini
  `thoughtSignature`) are mutually non-interchangeable. They are preserved
  byte-for-byte same-protocol (fast path) but **never forwarded to a different
  provider** — forwarding a foreign blob causes a hard 400.
- **Provider-specific params.** Params the IR doesn't model are dropped on
  cross-protocol requests (they'd be rejected by the target API). Same-protocol
  requests keep everything via the fast path.
- **Responses statefulness.** `previous_response_id` and server-side
  conversation state have no portable equivalent and are dropped cross-protocol
  (cost/latency degrades, correctness does not).
- **Prompt caching.** Anthropic `cache_control` breakpoints are carried through
  the IR (`CacheMarkers` on the request). Rendering back to Anthropic re-emits
  them on the tools/system/message tail; rendering to OpenAI or Gemini drops the
  marker because their prefix/implicit caching is automatic (no wire markup).
  Response usage is normalized across providers: the IR `Usage.input_tokens` is
  the inclusive total, with `cache_read_input_tokens` / `cache_creation_input_tokens`
  carried separately and echoed back in each protocol's native shape
  (`prompt_tokens_details.cached_tokens`, `input_tokens_details.cached_tokens`,
  `cachedContentTokenCount`). Anthropic→Anthropic keeps everything via the fast
  path. Explicit Gemini `CachedContent` (a stateful resource) is out of scope.
- **Gemini tool ids.** Gemini function calls have no id; tool results are keyed
  back to calls by function name.
- **Turn alternation.** Anthropic and Gemini require user/assistant turns to
  alternate, but parallel tool results arrive as separate messages. Rendering to
  those protocols coalesces consecutive same-role turns into one (e.g. multiple
  tool results become a single user turn with several `tool_result` blocks).
- **Parallel tool calls without an explicit tool_choice.** `parallel_tool_calls:
  false` can only be expressed via Anthropic's `tool_choice.disable_parallel_tool_use`.
  When a request carries no `tool_choice`, there is nowhere to attach it and the
  disable intent is dropped (semantics degrade, no 400).
- **Unterminated trailing SSE.** Incremental SSE decoding emits events on `\n\n`
  boundaries; if an upstream stream ends without the final `\n\n`, the leftover
  buffer is discarded. Conformant providers always terminate cleanly, so this only
  affects malformed streams.
- **Upstream errors** (non-2xx) are passed through unchanged, not translated.
