# Architecture

litellm-rust is a low-overhead gateway. A request flows through four layers:

```
                    ┌─────────────────────────────────────────────┐
  POST /v1/messages │  litellm-rust                                │
  ────────────────► │                                             │
                    │  endpoint ─► router ─► transformation ─► llm │ ──► provider API
  ◄──────────────── │                                             │ ◄──
       response     └─────────────────────────────────────────────┘
```

| Layer | File | Responsibility |
|---|---|---|
| **Endpoint** | `http/messages.rs`, `http/chat_completions.rs`, `http/responses.rs`, `http/gemini.rs` | Receive the request, authenticate, set the inbound protocol |
| **Router** | `sdk/router.rs` | Map the public model name to an upstream deployment (provider + wire format) |
| **Pipeline / codecs** | `http/pipeline.rs`, `sdk/codec/` | Translate inbound → IR → outbound (and back) between the four wire protocols |
| **LLM API** | `http/llm.rs` | The only place that does outbound networking |

The proxy speaks four wire protocols inbound and outbound and converts between
any pair (tool calling and streaming included). See
[protocols.md](protocols.md).

## Two halves

The code is split so the translation logic can ship as a standalone SDK,
independent of the proxy server around it:

| Half | Folders | What it is |
|---|---|---|
| **Translation layer** (future SDK) | `providers/` | Provider handlers + the router that picks one. Pure request/response shaping — no auth, no server state. |
| **Proxy server** | `proxy/`, `http/`, `cli/` | Everything around the translation: config loading, master-key auth, shared `AppState`, HTTP endpoints, the CLI wizard. |

`errors.rs` (the shared `GatewayError`) sits at the crate root — both halves use
it. The rule: `providers/` must not depend on `proxy/`. (One bridge remains:
`router.rs::from_config` reads `proxy::config::GatewayConfig`; when the SDK is
extracted, the proxy will build the route table and hand `providers/` plain
data instead.)

## Request flow

A request like:

```bash
curl http://localhost:4000/v1/messages \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $LITELLM_MASTER_KEY" \
  -d '{"model": "claude-opus-4-6", "messages": [...]}'
```

1. **Endpoint** (`http/messages.rs`) — `proxy::auth` checks the `Authorization: Bearer` token against the configured master key, parses the body, reads `model`, and tags the inbound protocol (here, Anthropic Messages).
2. **Router** (`sdk/router.rs`) — looks up `"claude-opus-4-6"` in the route table built at boot from `config.yaml`. Returns a `Route` whose deployment carries the outbound wire format.
3. **Pipeline** (`http/pipeline.rs`) — if inbound and outbound protocols match, rewrites the model alias and outbound headers and passes the body through (fast path). Otherwise it parses the body to the IR with the inbound codec and renders it with the outbound codec (`sdk/codec/`). See [protocols.md](protocols.md).
4. **LLM API** (`http/llm.rs`) — sends to `https://api.anthropic.com/v1/messages`, streaming the (possibly re-encoded) response back to the client.

## Config → routes

Config types, parsing, env expansion, and boot-time validation all live in
`proxy/config.rs`. The route table comes from `config.yaml`:

```yaml
model_list:
  - model_name: claude-opus-4-6           # ← public name, the lookup key
    litellm_params:
      model: anthropic/claude-opus-4-6     # ← provider_id / upstream_model
      api_key: os.environ/ANTHROPIC_API_KEY
```

At boot, each entry becomes a `Deployment`:

```
provider_id:    "anthropic"
upstream_model: "claude-opus-4-6"
api_base:       "https://api.anthropic.com"   # provider default, or api_base override
api_key:        "sk-ant-..."
```

This is what separates the public alias from the real upstream call.

## Config-defined agents

`config.yaml` can also define agents under `agents`, with sandbox selection and
E2B parameters configured under `general_settings`. Config parsing and
validation still flow through `proxy/config.rs`; agent-specific config types
live in `src/agents/config.rs`.

Agent runs are HTTP-triggered and streamed per run:

```bash
POST /api/agents/{agent_id}/run
GET  /events
```

The run endpoint returns `202` with an `event_url`. The `/events` endpoint is
the SSE stream for agent runs and emits `agent.run.started`,
`agent.sandbox.created`, `agent.output.delta`, and terminal run events. Event
payloads include `agent_id` and `run_id` so clients can filter the stream.

Sandbox provisioning is owned by the proxy. The agent does not receive a
sandbox provisioning tool; `src/agents/sandboxes/e2b.rs` creates the E2B
sandbox, starts the Claude Code process, streams process output, and terminates
the sandbox when the run ends.

## Providers are self-contained

Each provider is one folder under `src/sdk/providers/`. `build.rs` scans for any
subdirectory with a `mod.rs` and wires it in automatically — no edits anywhere
else in the tree.

To add a provider (e.g. OpenAI):

```
src/sdk/providers/openai/
└── mod.rs   # registry.register("openai", "https://api.openai.com", WireFormat::OpenAiResponses)
```

A provider just maps an id to a default API base and a wire format. The actual
protocol translation lives in the codec for that wire format (`src/sdk/codec/`),
shared across every provider that speaks it. The router, endpoint, and networking
layers never change.

**Rule:** the translation layer (`sdk/`) shapes protocols only. It never makes
network calls — all outbound HTTP lives in `http/llm.rs`. This keeps the hot path
in one place and stops each codec from re-implementing (and mis-implementing)
networking.

## Boot sequence

`main.rs` → `serve_gateway`:

1. Load + validate `config.yaml` (`proxy::config::load_config`)
2. Build the `ProviderRegistry` (`register_all`, generated by `build.rs`)
3. Build the `Router` from config + registry
4. Assemble `AppState` (config, router, one shared HTTP client)
5. Start the Axum server

`main.rs` also dispatches the `claude` CLI wizard and `logout` before serving
(see `cli/`).
