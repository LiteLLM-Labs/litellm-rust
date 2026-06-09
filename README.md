# litellm-rust
a simple and blazing fast LiteLLM-compatible ai gateway for coding agents (Claude Code, Codex, Hermes, etc.) 

[![Discord](https://img.shields.io/badge/Discord-Chat-5865F2?logo=discord&logoColor=white)](https://discord.gg/Nkxw3rm3EE)

![LLM Gateway Proxy Overhead](docs/benchmark.png)

## Usage with Claude code

```bash
lite claude
```

The first run prompts for your LiteLLM URL and API key, saves them to
`~/.config/lite/claude.env`, and starts Claude Code with:

```bash
ANTHROPIC_BASE_URL="https://your-litellm-rust-server.com"
ANTHROPIC_AUTH_TOKEN="$LITELLM_API_KEY"
```

Arguments after `lite claude` are forwarded to Claude Code:

```bash
lite claude --help
lite claude --model claude-sonnet-4-5
```

Run `lite claude --reset` to ignore saved settings and enter them again.

## Usage with Codex

```bash
lite codex
```

The first run prompts for your LiteLLM URL and API key, saves them to
`~/.config/lite/codex.env`, and starts Codex pointed at the gateway. Codex uses
the OpenAI **Responses API** (SSE over HTTP — no WebSocket), so requests land on
`POST /v1/responses`. The wizard injects the gateway via `-c` config overrides
and never edits your `~/.codex/config.toml`:

```bash
codex \
  -c model_provider="litellm" \
  -c model_providers.litellm.base_url="https://your-litellm-rust-server.com/v1" \
  -c model_providers.litellm.wire_api="responses" \
  -c model_providers.litellm.env_key="LITELLM_API_KEY"
# LITELLM_API_KEY is exported from your saved key
```

Arguments after `lite codex` are forwarded to Codex:

```bash
lite codex exec "fix the failing test"
lite codex -m gpt-5.5
```

Run `lite codex --reset` to ignore saved settings and enter them again.

The gateway needs an OpenAI model route in its config:

```yaml
model_list:
  - model_name: openai/*
    litellm_params:
      model: openai/*
      api_key: os.environ/OPENAI_API_KEY
      api_base: https://api.openai.com
```

**Codex Mac app:** the desktop app reads `~/.codex/config.toml`, so route it by
adding a provider block there (same fields the wizard passes), then select it in
the app:

```toml
model_provider = "litellm"

[model_providers.litellm]
name = "LiteLLM"
base_url = "https://your-litellm-rust-server.com/v1"
wire_api = "responses"
env_key = "LITELLM_API_KEY"
```

> Installing/updating the CLI: `cargo install --path . --force` so the `lite` on
> your `PATH` includes the `codex` subcommand (a stale install errors with
> `unrecognized subcommand 'codex'`).

## Quickstart

litellm-rust is compatible with your existing litellm config.yaml and DB. 

```yaml
model_list:
	- model_name: anthropic/*
		litellm_params:
			model: anthropic/*
			api_key: os.environ/ANTHROPIC_API_KEY


general_settings:
	master_key: os.environ/MASTER_KEY
	sandbox_choice: "e2b" # can be either "e2b" or "daytona"  
	e2b_sandbox_params:
		e2b_api_key: os.environ/E2B_API_KEY
		e2b_template: "litellm-4gb"
```

```
$ litellm-rust --config /app/config.yaml
```

## Deployment

Run it locally or deploy the UI + API to a host (e.g. Render) — see
[deployment.md](deployment.md) for step-by-step instructions, the required env
vars, and field-tested gotchas.

## Routes

```txt
POST /messages
POST /responses
POST /realtime
POST /audio
```

## Providers
- OpenAI
- Azure OpenAI
- VertexAI
- Bedrock

## Codebase map

Entry points and what runs at startup:

- **`src/main.rs`** — binary entry point. Parses CLI args, loads `config.yaml`, builds the HTTP client, calls `model_prices::load()`, then wires everything into `AppState` and starts the server.
- **`src/model_prices.rs`** — fetches the LiteLLM model cost/capability map from upstream at startup; falls back to the embedded `model_prices_backup.json` snapshot if the network is unavailable. Returns a `ModelCostMap`; `main.rs` stores it on `AppState`. Override the URL with `LITELLM_MODEL_COST_MAP_URL`.
- **`src/errors.rs`** — typed error enum. All error variants map to HTTP status + JSON body in one place.

Subsystems:

- **`src/http/`** — HTTP layer only. Route registration, auth, body extraction, response shaping. No business logic.
- **`src/sdk/routing.rs`** — request/model routing (maps model name → deployment + handler).
- **`src/sdk/transformations/`** — base transformation traits for endpoint families (`anthropic_messages`, `openai_responses`) and runtime adapters.
- **`src/sdk/providers/`** — provider-owned endpoint translations and runtime adapters (`anthropic/anthropic_messages`, `anthropic/runtime`, `cursor/runtime`, etc.).
- **`src/sdk/agents/`** — unified managed-agent runtime SDK (`Lap`) resources and types.
- **`src/proxy/`** — config loading, master-key auth, `AppState`.
- **`src/cli/`** — `lite claude` wizard: credential storage, model selector, Claude Code launcher.

## Coding standards

See [CODING_STANDARDS.md](CODING_STANDARDS.md).
