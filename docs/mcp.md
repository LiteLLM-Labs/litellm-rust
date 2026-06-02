# MCP Support

litellm-rust acts as an MCP gateway — it forwards requests from AI clients to your MCP servers, injecting upstream auth along the way. The config is **LiteLLM-compatible**: the `mcp_servers` block uses the same dict-keyed shape as the LiteLLM Python proxy, so an existing LiteLLM MCP config drops in for the supported cases.

## Quick Start

**1. Add servers to `config.yaml`** (keyed by server name):

```yaml
mcp_servers:
  linear:
    url: https://mcp.linear.app/mcp
    auth_type: bearer_token
    auth_value: os.environ/LINEAR_MCP_API_KEY
```

**2. Start the gateway, then point your client at:**

```
http://localhost:4000/mcp/linear
```

A public server needs no auth at all:

```yaml
mcp_servers:
  deepwiki_mcp:
    url: https://mcp.deepwiki.com/mcp
```

---

## How Do I Call It?

Two endpoints:

| Endpoint | When to use |
|---|---|
| `/mcp/{server_name}` | Target one server by its config key |
| `/mcp` | Single-server configs — gateway auto-selects. With >1 server, send `x-litellm-mcp-server: <name>` or `?server=<name>` |

All requests need your gateway master key:

```
Authorization: Bearer your-master-key
```

Methods: `GET`, `POST`, `DELETE` (streamable HTTP transport).

---

## Authentication

`auth_type` + `auth_value` map to an upstream header exactly as LiteLLM's managed transport does:

| `auth_type` | Header sent upstream |
|---|---|
| `none` (default) | *(no auth header)* |
| `api_key` | `X-API-Key: <auth_value>` |
| `bearer_token` | `Authorization: Bearer <auth_value>` |
| `basic` | `Authorization: Basic <base64(auth_value)>` |
| `authorization` | `Authorization: <auth_value>` (verbatim) |
| `token` | `Authorization: token <auth_value>` |

`auth_value` accepts `os.environ/VAR` expansion. The alias `authentication_token` is also accepted (matches LiteLLM).

### Extra headers

```yaml
mcp_servers:
  docs:
    url: https://mcp.example.com/mcp
    static_headers:            # always sent upstream
      x-workspace: prod
    extra_headers:             # names of inbound request headers to forward
      - x-trace-id
```

- `static_headers` — fixed key/value pairs always sent upstream (override auth/forwarded on conflict).
- `extra_headers` — a list of inbound header **names** the gateway forwards from the client request. Credential headers (`authorization`, `x-api-key`, `x-litellm-*`) are **never** forwarded, even if listed, so your gateway master key cannot leak to a third-party server.

---

## Claude Code SDK Example

```js
import { query } from "@anthropic-ai/claude-code";

for await (const msg of query({
  prompt: "List available tools",
  options: {
    mcpServers: {
      gateway: {
        type: "http",
        url: "http://localhost:4000/mcp/linear",
        headers: { Authorization: "Bearer your-master-key" },
      },
    },
  },
})) {
  console.log(msg);
}
```

---

## Known gaps vs LiteLLM

These LiteLLM features are **not yet supported**. Where a config uses them, the gateway fails fast (config error or a loud startup warning) rather than misbehaving silently:

- **Multi-server aggregation behind a single `/mcp`.** LiteLLM merges every server's tools behind one endpoint; litellm-rust serves one server per request. Target `/mcp/{name}` (or send `x-litellm-mcp-server` / `?server=`). A multi-server config logs a startup warning.
- **Per-request upstream creds via `x-mcp-{server}-{header}`.** Configure `auth_value` server-side instead.
- **`oauth2`, `oauth2_token_exchange`, `aws_sigv4` auth types** — rejected at config load.
- **`sse` and `stdio` transports** — only `http` (streamable HTTP) is served; others rejected at config load.

---

## FAQ

**Environment variables in config?**

```yaml
mcp_servers:
  linear:
    url: os.environ/LINEAR_MCP_URL
    auth_type: bearer_token
    auth_value: os.environ/LINEAR_MCP_API_KEY
```
`url`, `auth_value`, and `static_headers` values all support `os.environ/VAR`.

**I'm upgrading from the old list format and get a config error.**
`mcp_servers` changed from a list (`- id: x`) to a dict keyed by server name (`x:`). Move the `id` up to the map key and rename `api_key`→`auth_value` (+ set `auth_type`), `headers`→`static_headers`.

**Does the gateway read MCP messages?**
No. Pure pass-through — JSON-RPC is handled by the upstream and client. The gateway only handles routing + auth.

---

## Per-user credentials (BYOK)

Some MCP servers act on behalf of an individual user (Gmail, Linear, Notion).
Instead of one shared `auth_value`, set `is_byok: true` so each user supplies
their own upstream credential. The gateway stores it **per user, encrypted at
rest** (AES-256-GCM, key derived from `master_key`) and injects it into that
user's MCP calls using the server's `auth_type` mapping.

BYOK requires `general_settings.master_key` and `database_url`.

```yaml
mcp_servers:
  gmail:
    url: os.environ/GMAIL_MCP_URL
    auth_type: bearer_token       # how the per-user credential is injected
    is_byok: true
    byok_description:
      - "Gmail OAuth access token"
```

### Setup via API

```bash
# 1. Admin mints a user API key
curl -X POST $BASE/user/new \
  -H "Authorization: Bearer $MASTER_KEY" \
  -d '{"user_alias":"alice"}'
# → {"user_id":"user_…","key":"sk-…"}

# 2. The user stores their own token (static / BYOK)
curl -X POST $BASE/v1/mcp/server/gmail/user-credential \
  -H "Authorization: Bearer $USER_KEY" \
  -d '{"credential":"ya29.<gmail-token>"}'

# …or an OAuth2 token set
curl -X POST $BASE/v1/mcp/server/gmail/oauth-user-credential \
  -H "Authorization: Bearer $USER_KEY" \
  -d '{"access_token":"ya29.…","refresh_token":"1//…","expires_in":3599}'

# 3. The user calls the MCP server; the gateway injects their token upstream
curl -X POST $BASE/mcp/gmail \
  -H "Authorization: Bearer $USER_KEY" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| POST | `/user/new` | master key | create a user + API key |
| POST | `/key/generate` | master key | mint another key for a user |
| GET | `/v1/mcp/user-credentials` | user key | list the caller's stored credentials |
| POST/GET/DELETE | `/v1/mcp/server/{id}/user-credential` | user key | set / check / remove a static token |
| POST/DELETE | `/v1/mcp/server/{id}/oauth-user-credential` | user key | set / remove an OAuth2 token set |
| GET | `/v1/mcp/server/{id}/oauth-user-credential/status` | user key | OAuth credential status |

A BYOK call with no stored credential returns `401` with a message pointing at
the `user-credential` endpoint. The master key is not a user, so it cannot call
BYOK servers directly — create a user key first.

### Known gaps vs LiteLLM
- **Token refresh** — an expired OAuth access token is sent as-is; automatic
  refresh via the refresh token is not yet implemented.
- **Browser OAuth flow** — the `/{server}/authorize` redirect dance is not
  implemented; users paste a token obtained out-of-band.
- **Key hashing** — user API keys are stored verbatim (LiteLLM hashes them).
- **DB-managed servers** — servers are defined in `config.yaml`; the
  `POST /v1/mcp/server` registry API is not implemented.
