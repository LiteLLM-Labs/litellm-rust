# Coding Standards

This repo is a low-overhead gateway. Keep the code explicit, boring, and easy
to profile.

## Architecture

- Core gateway behavior lives under `src/ai_gateway/`. HTTP is only one
  adapter into that core.
- HTTP handlers only do protocol work: auth, body extraction, response shaping.
- Model lookup and provider calls live in `llms/`.
- Provider request/response shape changes live behind `MessagesTransformation`;
  every provider owns its own transformation implementation.
- Provider detection follows the LiteLLM `provider/model` convention through
  `get_custom_llm_provider`.
- SDK-style gateway entrypoints use request/response structs under
  `src/ai_gateway/`; HTTP routes call those entrypoints instead of duplicating
  gateway logic.
- Config parsing lives in `config/`; never read env vars or YAML from handlers.
- Add a provider by adding a provider module, not by branching through the app.
- Prefer boot-time validation over runtime surprises.

## Performance

- Reuse one HTTP client from `AppState`; never create clients per request.
- Parse request JSON once. Re-serialize only when the upstream model must be
  rewritten from a public alias.
- Streaming responses are byte passthrough. Do not parse SSE chunks on the hot
  path unless a feature explicitly requires it.
- Add callbacks around the transformation/client boundary, not inside provider
  transformation code. Providers should translate protocol shape; callbacks
  should observe or post-process traffic.
- Keep middleware minimal and measurable.
- Avoid heap-heavy abstractions in request routing. Clear functions beat clever
  generic frameworks here.

## Rust Style

- Modules are small and named by responsibility.
- Public types have narrow fields and no `serde_json::Value` escape hatches
  unless the endpoint is intentionally pass-through.
- Errors use typed enums and map to HTTP responses in one place.
- No `unwrap()` or `expect()` in request paths.
- Tests cover config resolution, auth, non-stream forwarding, and streaming
  forwarding before adding new behavior.

## Compatibility

- `config.yaml` should remain LiteLLM-compatible where possible.
- v0 supports Anthropic only. Unsupported providers must fail at boot with a
  clear message.
- `/v1/messages` is the first stable route. Add new routes behind focused
  modules and tests.
