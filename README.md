# litellm-rust
a simple and blazing fast ai gateway for giving agents access to resources (LLM's, MCP's, API's). 

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

## Providers
- OpenAI
- Azure OpenAI
- VertexAI
- Bedrock
