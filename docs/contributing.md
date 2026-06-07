# Contributing

## Prerequisites

- Rust toolchain — [rustup.rs](https://rustup.rs)
- An Anthropic API key (for end-to-end testing)

## Run the server locally

**1. Clone and build**

```bash
git clone <repo>
cd litellm-rust
cargo build
```

**2. Create a config**

```bash
cp config.yaml.example config.yaml
```

`config.yaml.example` ships with a single Anthropic model entry. The `api_key`
and `master_key` fields read from environment variables by default:

```yaml
model_list:
  - model_name: claude-sonnet
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: os.environ/ANTHROPIC_API_KEY

general_settings:
  master_key: os.environ/LITELLM_MASTER_KEY
```

**3. Start the server**

```bash
export ANTHROPIC_API_KEY=sk-ant-...
export LITELLM_MASTER_KEY=sk-mykey

cargo run -- --config config.yaml
```

Server listens on `http://localhost:4000` by default.

**4. Verify**

```bash
# Health check
curl http://localhost:4000/health

# Chat request
curl -X POST http://localhost:4000/v1/messages \
  -H "Authorization: Bearer $LITELLM_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-sonnet", "messages": [{"role": "user", "content": "hello"}], "max_tokens": 10}'
```

The proxy speaks four wire protocols, inbound and outbound, and translates
between any pair (see [protocols.md](protocols.md)):

| Inbound endpoint | Protocol |
|---|---|
| `POST /v1/messages` | Anthropic Messages |
| `POST /v1/chat/completions` | OpenAI Chat Completions |
| `POST /v1/responses` | OpenAI Responses |
| `POST /v1beta/models/{model}:generateContent` (`:streamGenerateContent`) | Gemini |

The outbound protocol is the provider's (`anthropic` → Messages, `openai`/`codex`
→ Responses, `openai_chat` → Chat Completions, `gemini` → Gemini), overridable
per model with `litellm_params.wire_api`.

## Run tests

```bash
cargo test
```

Integration tests in `tests/` spin up a local wiremock server — no real API
calls needed.

## Add a provider

A provider just maps an id to a default API base and wire format. Drop a folder
under `src/sdk/providers/`:

```
src/sdk/providers/openai/
└── mod.rs   # registry.register("openai", "https://api.openai.com", WireFormat::OpenAiResponses)
```

`build.rs` auto-discovers the folder and wires it in. No other files need
editing. See `src/sdk/providers/anthropic/` for a reference.

If the provider speaks a wire format the gateway doesn't have yet, add a codec
(see below) and a `WireFormat` variant.

## Add a protocol (codec)

Protocol translation lives in `src/sdk/codec/`. Each wire format implements
`ProtocolCodec` — parse its shape into the canonical IR (`ir.rs`) and render the
IR back out — so converting between N protocols needs N codecs, not N×N. See
[protocols.md](protocols.md) for the design and `src/sdk/codec/anthropic.rs` for
a reference.

## Project layout

```
src/
  sdk/
    codec/      # protocol codecs + canonical IR (pure translation)
    providers/  # provider id → (api base, wire format)
    router.rs   # model name → deployment
  proxy/        # config, master-key auth, AppState
  http/         # axum endpoints, request pipeline, outbound HTTP (http/llm.rs)
  cli/          # CLI wizard
  errors.rs     # shared GatewayError
```

See [architecture.md](architecture.md) for the full request flow.
