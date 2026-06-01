# LiteLLM Gateway v2

The AI gateway for agent servers.

The first AI gateways were built for chat completions: route a prompt to a
model, log the tokens, maybe retry on failure.

The next AI gateways have a bigger job.

People are building agent servers. Those servers do not just call one model.
They stream long-running work, call tools, talk to MCP servers, switch providers,
touch internal APIs, and need policy on every step.

That makes the gateway the product surface. It is where teams decide which model
an agent can use, which tools it can touch, how traffic fails over, how costs are
tracked, and how every action is observed.

LiteLLM Gateway v2 is built for that world: one fast, provider-neutral control
plane for model and agent traffic.

![LLM Gateway Proxy Overhead](benchmark.png)

## Why this exists

- **Agent servers need a real gateway**: LLMs, MCP servers, realtime streams,
  audio, and internal APIs need one place for routing, policy, and observability.
- **The gateway is now on the hot path**: every token, tool call, retry, provider
  switch, and streaming response passes through it.
- **Performance matters more than before**: agent requests fan out into many
  downstream calls, so proxy overhead compounds quickly.
- **Teams need continuity**: existing LiteLLM config and database setup should
  carry forward into the v2 gateway.

## What v2 means

LiteLLM Gateway v2 is not just a faster proxy. It is the control plane for
production agent traffic:

- **One gateway for AI traffic**: messages, responses, realtime, audio, tools,
  and provider APIs.
- **Provider-neutral routing**: OpenAI, Azure OpenAI, Vertex AI, Bedrock, and
  more behind one interface.
- **Policy at the boundary**: centralize access, budgets, retries, failover, and
  auditability outside the agent server.
- **Fast data plane**: implemented in Rust so the gateway can sit in front of
  high-volume agent traffic without becoming the bottleneck.
- **LiteLLM-compatible migration**: keep the `config.yaml` and DB shape teams
  already use.

## Usage with Claude code

```bash
export ANTHROPIC_BASE_URL="https://your-litellm-rust-server.com"
export ANTHROPIC_AUTH_TOKEN="$LITELLM_API_KEY"

claude
```

## Quickstart

`litellm-rust` is compatible with your existing LiteLLM `config.yaml` and DB.

```yaml
model_list:
  - model_name: gpt-4o
    litellm_params:
      model: azure/my_azure_deployment
      api_base: os.environ/AZURE_API_BASE
      api_key: "os.environ/AZURE_API_KEY"
      # [OPTIONAL] LiteLLM uses the latest Azure api_version by default.
      api_version: "2025-01-01-preview"
```

```bash
litellm-rust --config /app/config.yaml
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
