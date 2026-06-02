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

```bash
# git clone https://github.com/LiteLLM-Labs/litellm-rust
$ cargo run -- --config config.yaml
```

## Routes

```txt
POST /messages
```

## Providers
- Anthropic

## Coding standards

See [CODING_STANDARDS.md](CODING_STANDARDS.md).
