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
- **`src/providers/`** — provider registry, per-provider request/response transformation, model router (maps model name → deployment + handler).
- **`src/proxy/`** — config loading, master-key auth, `AppState`.
- **`src/cli/`** — `lite claude` wizard: credential storage, model selector, Claude Code launcher.

## Coding standards

See [CODING_STANDARDS.md](CODING_STANDARDS.md).
