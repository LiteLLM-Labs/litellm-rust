# litellm-rust

a simple and blazing fast ai gateway for agents

litellm-rust is a small gateway for OpenAI-compatible traffic. no proxy framework,
no extra surface area, and no complex setup. just the core routes most apps need.

## Features

- Rust-based gateway
- OpenAI-compatible `/messages`, `/responses`, `/realtime`, and `/audio` routes
- Small deployment surface
- Built for low latency and simple ops

## Installation

```bash
git clone https://github.com/LiteLLM-Labs/litellm-rust
cd litellm-rust
cargo run
```

## Usage

```bash
curl http://localhost:4000/messages \
  -H "authorization: Bearer $LITELLM_API_KEY" \
  -H "content-type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "messages": [
      { "role": "user", "content": "hello" }
    ]
  }'
```

## Usage with Claude code

```bash
export ANTHROPIC_BASE_URL="https://your-litellm-rust-server.com"
export ANTHROPIC_AUTH_TOKEN="$LITELLM_API_KEY"

claude
```

## Routes

```txt
POST /messages
POST /responses
POST /realtime
POST /audio
```

## About

litellm-rust is an experimental rewrite focused on the smallest useful gateway:
the main model, realtime, and audio APIs, implemented in Rust.

## License

MIT
