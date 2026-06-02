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

## Coding standards

See [CODING_STANDARDS.md](CODING_STANDARDS.md).
