# src/

Source layout for the litellm-rust gateway. A request flows
`endpoint → router → pipeline/codec → llm api`; see
[docs/architecture.md](../docs/architecture.md) and
[docs/protocols.md](../docs/protocols.md) for the full picture.

The codebase splits into two halves: the **translation layer** (`sdk/`),
designed to ship as a standalone SDK, and the **proxy server** (`proxy/`) around
it. The translation layer must not depend on proxy internals.

## Entrypoints

| File | What it does |
|---|---|
| `main.rs` | Binary entry. Parses args, loads config, builds the provider registry + router, starts the server. Also dispatches the `claude` CLI wizard. |
| `lib.rs` | Crate root. Declares the public modules below. |
| `errors.rs` | `GatewayError` — the shared error type, mapped to HTTP responses in one place. Used by both halves, so it lives at the top level. |

## Folders

| Folder | Responsibility |
|---|---|
| `sdk/` | **Translation layer (future SDK).** `codec/` holds the canonical IR and one codec per wire protocol (Anthropic / OpenAI Chat / OpenAI Responses / Gemini); `providers/` maps each provider id to a default API base + wire format (auto-discovered at build time); `router.rs` maps model names → upstream deployment. Translates request shape only — never makes network calls. |
| `proxy/` | **Proxy-server concerns**, kept out of the SDK: `config.rs` (`config.yaml` parse + env expansion + validation), `state.rs` (`AppState` — config, router, shared HTTP client), `auth/` (master-key check). |
| `http/` | HTTP layer. Routes (`routes.rs`), the protocol endpoints (`messages.rs`, `chat_completions.rs`, `responses.rs`, `gemini.rs`), the translation `pipeline.rs`, health check, and `llm.rs` — the **only** place that does outbound networking to providers. |
| `cli/` | The `litellm-rust claude` wizard: configures Claude Code to point at the gateway (arg parsing, credential storage, terminal prompts). |

## Adding a provider

Drop a folder under `sdk/providers/<name>/` with a `mod.rs` (`pub fn init`) that
registers the id, default API base, and wire format. `build.rs` wires it in
automatically — no edits anywhere else. To add a new wire protocol, add a codec
under `sdk/codec/`. See
[docs/protocols.md](../docs/protocols.md) and
[docs/architecture.md](../docs/architecture.md#providers-are-self-contained).
