# litellm-rust
a simple and blazing fast ai gateway for giving agents access to resources (LLM's, MCP's, API's). 

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
  - model_name: gpt-4o
    litellm_params:
      model: azure/my_azure_deployment
      api_base: os.environ/AZURE_API_BASE
      api_key: "os.environ/AZURE_API_KEY"
      api_version: "2025-01-01-preview" # [OPTIONAL] litellm uses the latest azure api_version by default
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
