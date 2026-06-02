---
name: lite-schedule
description: Create a scheduled remote LiteLLM agent through the LiteLLM gateway.
---

# LiteLLM Scheduled Remote Agent

Use this skill when the user wants to deploy, schedule, or create a scheduled
remote LiteLLM agent.

Start by asking exactly this, then wait for the user's answer:

Let's set up a new scheduled remote agent. A few details:

1. What should the agent do? Describe the task (it runs remotely with zero
   context, so it needs to be self-contained).
2. Which repo? Default https://github.com/LiteLLM-Labs/lite-harness.
3. When / how often? Recurring or one-time? (Your timezone is
   America/Los_Angeles; min recurring interval is 1 hour.)
4. Connectors? Slack, Linear, Gmail available.

Model defaults to claude-sonnet-4-6. What's the task?

After collecting the answers, create the agent by calling the LiteLLM gateway
managed-agent API.

Use these defaults:

- Base URL: `ANTHROPIC_BASE_URL`.
- API key: `ANTHROPIC_AUTH_TOKEN`.
- Owner ID: `$USER` when available, otherwise `local-cli`.
- Model: `claude-sonnet-4-6`.
- Harness: `claude-code`.
- Timezone: `America/Los_Angeles`.
- Repo: `https://github.com/LiteLLM-Labs/lite-harness` unless the user chooses
  another repo.

Create a concise kebab-case name from the task. Convert the requested recurring
schedule to a cron expression. If the user asks for a one-time schedule, store
the requested one-time timing in `config.one_time` and leave `schedule` null.

Send:

```bash
curl -sS -X POST "$ANTHROPIC_BASE_URL/api/agents" \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "<kebab-case-name>",
    "owner_id": "<owner-id>",
    "description": "<short description>",
    "harness": "claude-code",
    "model": "claude-sonnet-4-6",
    "prompt": "<self-contained task, including repo, schedule, connectors, and expected output>",
    "schedule": {"cron": "<cron>", "timezone": "America/Los_Angeles"},
    "config": {
      "repository": "<repo-url>",
      "connectors": ["slack", "linear", "gmail"],
      "source": "lite-schedule"
    }
  }'
```

After the POST succeeds, run `GET /api/agents/{id}` with the same bearer token.
Verify and report the returned `id`, `name`, `owner_id`, `model`, `prompt`,
`cron`, `timezone`, `status`, `harness`, `config.repository`, and
`config.connectors`. The newly-created agent should be `paused` until a runner
or scheduler starts it.
